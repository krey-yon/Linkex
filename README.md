# tross — LinkedIn Profile API

A read-only HTTP service for public LinkedIn member profiles. It speaks
LinkedIn's Voyager web API **server-side**: given a profile URL, it fetches
the member graph the way a signed-in browser would, parses it, and returns a
clean, stable JSON schema. No browser, no DOM scraping, no headless browser —
just one authenticated upstream client, a cache, and an HTTP endpoint.

Profiles are cached (TTL + disk persistence) and the upstream is throttled
and circuit-broken, so one LinkedIn account is all you need.

## Quick start (local)

```sh
cp .env.example .env
```

Open a signed-in browser session on linkedin.com, open DevTools → Network →
any `voyager` request, right-click → **Copy → Copy request headers**, and
paste the `Cookie:` header value into `.env`:

```
LINKEDIN_COOKIE_HEADER=li_at=AQED...; JSESSIONID="ajax:...
```

Start the server (defaults: port 8000, no API key required locally):

```sh
cargo run
```

Try it:

```sh
curl -H 'X-API-Key: ...' \
  'http://127.0.0.1:8000/v1/profile?url=https://www.linkedin.com/in/<vanity>/'
```

## Routes

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/` | no | Service banner: name, version, public routes |
| GET | `/healthz` | no | Liveness: process + HTTP stack up (never calls LinkedIn) |
| GET | `/readyz` | no | Readiness: upstream session state (reports the reason when not ready) |
| GET | `/v1/account` | yes | Current API-key balance and cache hit/miss prices |
| GET | `/v1/profile?url=&refresh=` | yes | Profile by URL; `refresh=true` bypasses the cache |
| POST | `/v1/profile` | yes | Same, JSON body: `{"url": "...", "refresh": false}` |
| GET | `/v1/profile/raw` | yes | Raw upstream payload; only when `EXPOSE_RAW_ENDPOINT=true` |
| GET | `/v1/session` | yes | Session diagnostics (age, expiry, resource URL, probe info) |

## Authentication and credits

- Compose enables Redis billing with `BILLING_ENABLED=true` and seeds the 10
  demo accounts in `config/api_keys.json` without resetting existing balances.
- Send `X-API-Key: tross_sk_...` (or `Authorization: Bearer ...`). Redis stores
  a second SHA-256 digest of each key, not the presented key itself.
- Successful cache hits cost `$0.25`; successful misses cost `$0.50`. Failed
  requests are refunded. `X-Credit-Balance-Cents` and `X-Request-Cost-Cents`
  synchronize clients after every billable request.
- `GET /v1/account` validates a key and returns its current balance and prices.
- Legacy `API_KEYS` comma-separated auth remains available when Redis billing
  is disabled. With neither billing nor `API_KEYS`, the API is open for local dev.

> The included keys are deterministic SHA-256 hashes of known demo emails and
> are therefore guessable. Use cryptographically random keys and an untracked
> seed/administration path for a real production deployment.

## Response envelope

Every response (success and failure) is JSON:

```json
{
  "ok": true,
  "data": { "profile": { ... }, "meta": { ... } }
}
```

```json
{
  "ok": false,
  "error": { "code": "PROFILE_NOT_FOUND", "message": "...", "request_id": "..." }
}
```

Stable error codes:

| Code | Meaning |
|---|---|
| `VALIDATION_ERROR` | Bad query/body shape |
| `INVALID_PROFILE_URL` | Not a parseable LinkedIn profile URL |
| `API_KEY_MISSING` / `API_KEY_INVALID` | Request rejected by key auth |
| `RATE_LIMITED` | Your rate limit for this instance hit |
| `LINKEDIN_AUTH_FAILED` | Upstream login/session failure |
| `LINKEDIN_SESSION_EXPIRED` | Cookie jar is stale; refresh the browser session |
| `LINKEDIN_CHALLENGE_REQUIRED` | LinkedIn prompted a challenge — replace cookies, or switch account |
| `PROFILE_NOT_FOUND` | No such member |
| `PROFILE_NOT_VISIBLE` | Profile exists but is hidden from this session |
| `LINKEDIN_RATE_LIMITED` | Upstream throttled us; retry after cooldown |
| `LINKEDIN_UNAVAILABLE` | Upstream 5xx/network failure |
| `UPSTREAM_CIRCUIT_OPEN` | Circuit breaker tripped; temporary cooldown |
| `ENDPOINT_DISABLED` | Raw endpoint turned off |
| `ROUTE_NOT_FOUND` / `METHOD_NOT_ALLOWED` | Wrong path/method |
| `INTERNAL_ERROR` | Unexpected server error (bug) |

## Session setup options

1. **Whole Cookie header (recommended).** `LINKEDIN_COOKIE_HEADER` = the full
   `Cookie:` header value. Survives cookie renewal best.
2. **`LINKEDIN_LI_AT` + `LINKEDIN_JSESSIONID`.** The two cookies extracted
   from the same header.
3. **Email + password.** `LINKEDIN_EMAIL` / `LINKEDIN_PASSWORD`, only honored
   with `ALLOW_PASSWORD_LOGIN=true`. Off by default: password login is the
   path most likely to trigger challenges and account verification.

The session (and its device cookies — `bcookie`, `bscookie`, `liap` — which
keep the session alive) is persisted to `SESSION_STATE_PATH` (`/data/...` in
the container), so restarts keep the session. If `LINKEDIN_COOKIE_HEADER`
changes, delete the state file so the new credentials take effect.

## Operational notes

- **Rate limits.** `RATE_LIMIT_REQUESTS` per `RATE_LIMIT_WINDOW_SECONDS` per
  client IP. Inside limits, profile responses that are cached only cost an
  LRU lookup.
- **Upstream throttle + circuit breaker.** Request spacing
  (`UPSTREAM_MIN_INTERVAL_SECONDS` + jitter), bounded concurrency and
  retries; after `CIRCUIT_BREAKER_THRESHOLD` consecutive upstream failures the
  circuit opens for `CIRCUIT_BREAKER_COOLDOWN_SECONDS` and requests answer
  `UPSTREAM_CIRCUIT_OPEN` immediately instead of hammering LinkedIn.
- **Cache.** Profiles default to a 24-hour server-side TTL (`CACHE_TTL_SECONDS=86400`).
  API responses use `Cache-Control: private, no-store` so browsers and shared
  proxies cannot bypass authentication/billing. `CACHE_MAX_ENTRIES` bounds the
  in-memory cache; when
  `CACHE_PERSIST=true` entries are written to `CACHE_DIR` (see
  `load_disk`/TTL validation) — `/data/profiles` in the container.
- **Single-flight.** Concurrent requests for the same profile coalesce into
  one upstream fetch, keyed by URL.
- **One replica per credential.** The upstream account is a single identity;
  running multiple replicas with the same cookies multiplies session
  contention and risk of challenge. Run ONE replica behind a load balancer;
  `CACHE_DIR`/`SESSION_STATE_PATH` are bound to a single `/data` volume
  anyway.
- **Readiness vs liveness.** `/healthz` = the process answers HTTP (used by
  the container healthcheck). `/readyz` = the upstream session can still be
  used; probes should use `/healthz` for restart and `/readyz` for traffic
  routing.
- **Graceful shutdown.** SIGTERM/SIGINT drain in-flight requests, then exit.
- **Observability.** Structured logs (`LOG_FORMAT=json`); fields include
  `request_id`, method/path, status, latency, cache hits, upstream timing and
  error codes. Secrets (cookies, `LINKEDIN_*`, `PROXY_URL`) are redacted from
  `Debug` output and never logged.
- **Healthcheck.** `tross healthcheck` probes `http://127.0.0.1:$PORT/healthz`
  and exits non-zero on failure — used by the Docker `HEALTHCHECK`.

## Docker

```sh
docker build -t tross .
docker compose up -d          # production env: ENVIRONMENT=production, JSON logs
```

- Runs as **non-root** uid 10001; only `/data` is writable
  (`CACHE_DIR=/data/profiles`, `SESSION_STATE_PATH=/data/linkedin_session.json`).
- Named volume `api-data` keeps the profile cache and LinkedIn session across
  restarts. `redis-data` uses AOF persistence for API-key balances.
- Compose starts Redis, waits for its healthcheck, enables billing, pins the
  cache TTL to 24 hours, and sets `ENVIRONMENT=production` / `LOG_FORMAT=json`. It reads
  `.env` from the host, publishes `8000:8000`, healthchecks every 30s and
  caps the container at 512M memory.
- `docker compose logs --follow api`; rotate/refresh `LINKEDIN_COOKIE_HEADER`
  in `.env` and `docker compose up -d` again when the session expires.

## Development

```sh
cargo fmt --all --check   # formatting
cargo clippy --all-targets -- -D warnings
cargo test                # unit + integration tests
cargo bench --bench parser
```

Quality gates: clippy with warnings as errors, green test suite, and the
golden contract test over `fixtures/` (normalised + embedded Dash envelopes)
which pins the public JSON schema. `benches/parser.rs` times parse +
serialize of the golden `dash_normalized.json` fixture (no criterion
dependency; prints ms/iter and MB/s).

Dependency auditing: `cargo audit` (rustsec) is not installed in this
workspace; install it with `cargo install cargo-audit` and run `cargo audit`
before shipping a release.

## Legal

This service fetches public profiles through an authenticated LinkedIn
session. Use it only in a way that complies with LinkedIn's terms of service,
and with applicable privacy law: the data returned is personal data. Keep
`CACHE_TTL_SECONDS` short, and remember that `CACHE_PERSIST=true` retains
profile data in `/data` until it expires per instance TTL.
