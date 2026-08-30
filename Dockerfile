# syntax=docker/dockerfile:1
#
# Multi-stage build. The builder pins the toolchain to rust:1.92-bookworm;
# dependency compilation is cached with a manifest-first stub build, so only
# the tross crate itself recompiles when src/ changes.

# ---- builder --------------------------------------------------------------
FROM rust:1.92-bookworm AS builder
WORKDIR /app

# Cache layer: manifests first, stub sources, warm build of all dependencies
# (--locked honours the committed Cargo.lock; strip = true lives in Cargo.toml).
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src benches \
    && printf 'fn main() {}\n' > src/main.rs \
    && : > src/lib.rs \
    && printf 'fn main() {}\n' > benches/parser.rs \
    && cargo build --release --locked

# Real sources; only the tross crate (and its lib) recompile. `touch` forces a
# rebuild because the copied files can carry older mtimes than the stub build's
# fingerprint, which would otherwise make cargo consider the crate fresh and
# ship the stub binary.
COPY src ./src
RUN find src -name '*.rs' -exec touch {} + \
    && cargo build --release --locked

# ---- runtime --------------------------------------------------------------
FROM debian:bookworm-slim

# ca-certificates only (rustls TLS; no openssl library needed).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin appuser \
    && mkdir -p /data/profiles \
    && chown -R appuser:appuser /data

COPY --from=builder /app/target/release/tross /usr/local/bin/tross
# Static site served at `/` by ServeDir (relative path, resolved from CWD).
COPY site ./site
COPY config ./config

# The only writable path in the container is /data (HOME-style layout).
# All paths can be overridden via the usual environment variables.
ENV HOME=/data \
    PORT=8000 \
    CACHE_DIR=/data/profiles \
    SESSION_STATE_PATH=/data/linkedin_session.json

EXPOSE 8000

USER appuser

ENTRYPOINT ["tross"]

# `tross healthcheck` probes /healthz on the PORT env var of this container.
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD ["tross", "healthcheck"]
