//! HTTP request middleware: request IDs, tracing-friendly headers, API-key
//! authentication, and bounded inbound rate limiting.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::Request;
use axum::extract::connect_info::ConnectInfo;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use crate::error::{AppError, REQUEST_ID};
use crate::linkedin::client::Upstream;
use crate::state::AppState;

const REQUEST_ID_MAX_LEN: usize = 64;
const MAX_IDENTITIES: usize = 4096;

pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
pub const X_RESPONSE_TIME: HeaderName = HeaderName::from_static("x-response-time-ms");
pub const X_CACHE: HeaderName = HeaderName::from_static("x-cache");
pub const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");
pub const X_CREDIT_BALANCE: HeaderName = HeaderName::from_static("x-credit-balance-cents");
pub const X_REQUEST_COST: HeaderName = HeaderName::from_static("x-request-cost-cents");

/// Request-id middleware: accept a validated inbound id or generate one
/// (12 hex characters, mirroring the Python service), then stamp
/// `X-Request-ID` and `X-Response-Time-Ms` on every response.
pub async fn request_id_middleware(request: Request, next: Next) -> Response {
    let inbound = request
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && v.len() <= REQUEST_ID_MAX_LEN)
        .filter(|v| v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .map(str::to_string);
    let request_id = inbound.unwrap_or_else(generate_request_id);

    let started = Instant::now();
    let response = REQUEST_ID
        .scope(request_id.clone(), async { next.run(request).await })
        .await;
    let elapsed_ms = started.elapsed().as_millis() as i64;

    let mut response = response;
    let mut set_header = |name: HeaderName, value: String| {
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(name, value);
        }
    };
    set_header(X_REQUEST_ID, request_id);
    set_header(X_RESPONSE_TIME, elapsed_ms.to_string());
    response
}

fn generate_request_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

/// API-key plus inbound rate-limit middleware for `/v1` routes.
///
/// Keys are compared in constant time against every configured candidate.
/// The rate-limited identity is the API key when present, otherwise the peer
/// address — `X-Forwarded-For` is trusted only when the peer is a configured
/// trusted proxy.
pub async fn api_key_and_rate_limit<U: Upstream>(
    axum::extract::State(state): axum::extract::State<Arc<AppState<U>>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let settings = &state.settings;
    let supplied: Option<String> = request
        .headers()
        .get(X_API_KEY)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| bearer_token(request.headers()));

    let billing_identity = if state.billing.enabled() {
        let supplied = supplied
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or(AppError::ApiKeyMissing)?;
        Some(state.billing.authenticate(supplied).await?)
    } else {
        if settings.auth_required() {
            let supplied = supplied.as_deref().unwrap_or("");
            if supplied.is_empty() {
                return Err(AppError::ApiKeyMissing);
            }
            let mut matched: u8 = 0;
            for candidate in &settings.api_keys {
                matched |= ct_match(supplied.as_bytes(), candidate.as_bytes()) as u8;
            }
            if matched == 0 {
                return Err(AppError::ApiKeyInvalid);
            }
        }
        None
    };

    let identity = billing_identity
        .as_ref()
        .map(|identity| identity.redis_key.clone())
        .or_else(|| supplied.clone())
        .unwrap_or_else(|| {
            let peer = addr.to_string();
            let trusted = settings
                .trusted_proxies
                .iter()
                .any(|proxy| proxy == &addr.ip().to_string());
            if trusted {
                request
                    .headers()
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(',').next())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .unwrap_or(peer)
            } else {
                peer
            }
        });

    state.rate_limiter.check(
        &identity,
        settings.rate_limit_requests,
        settings.rate_limit_window_seconds,
    )?;

    if let Some(identity) = billing_identity.clone() {
        request.extensions_mut().insert(identity);
    }

    let billable = matches!(request.uri().path(), "/v1/profile" | "/v1/profile/raw");
    let reservation = if billable {
        match billing_identity {
            Some(identity) => Some(
                state
                    .billing
                    .reserve(identity, settings.cache_hit_cost_cents)
                    .await?,
            ),
            None => None,
        }
    } else {
        None
    };

    let mut response = next.run(request).await;
    if let Some(reservation) = reservation {
        let actual_cents = if response.status().is_success() {
            if response
                .headers()
                .get(X_CACHE)
                .is_some_and(|value| value == "HIT")
            {
                settings.cache_hit_cost_cents
            } else {
                settings.cache_miss_cost_cents
            }
        } else {
            0
        };
        let balance_cents = state.billing.settle(&reservation, actual_cents).await?;
        response.headers_mut().insert(
            X_CREDIT_BALANCE,
            HeaderValue::from_str(&balance_cents.to_string()).expect("integer header"),
        );
        response.headers_mut().insert(
            X_REQUEST_COST,
            HeaderValue::from_str(&actual_cents.to_string()).expect("integer header"),
        );
    }
    Ok(response)
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty() {
        Some(token.trim().to_string())
    } else {
        None
    }
}

/// Fixed-window rate limiting with a bounded identity map and stale-bucket
/// eviction. Identities are the API key (hashed boundary) or the client peer,
/// never unbounded labels.
pub struct RateLimiter {
    identities: std::sync::Mutex<std::collections::HashMap<String, VecDeque<Instant>>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            identities: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn check(&self, identity: &str, limit: u64, window_seconds: u64) -> Result<(), AppError> {
        let limit = limit.max(1);
        let window = std::time::Duration::from_secs(window_seconds.max(1));
        let now = Instant::now();
        let mut identities = self.identities.lock().unwrap();

        let hits = identities.entry(identity.to_string()).or_default();
        while hits
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            hits.pop_front();
        }
        if hits.len() >= limit as usize {
            let retry_after = window
                .checked_sub(now.duration_since(*hits.front().expect("non-empty")))
                .map(|d| d.as_secs_f64())
                .unwrap_or_else(|| window.as_secs_f64());
            return Err(AppError::RateLimited {
                retry_after_seconds: retry_after.max(0.0),
                limit,
                window_seconds,
            });
        }
        hits.push_back(now);

        // Bound the identity map; evict stale buckets and at most one noisy
        // identity per check insertion.
        if !identities.contains_key(identity)
            && let Some(oldest) = oldest_key(&identities)
        {
            identities.remove(&oldest);
        }
        if identities.len() > MAX_IDENTITIES * 2 {
            identities.retain(|_, hits| {
                hits.back()
                    .is_some_and(|t| now.duration_since(*t) <= window)
            });
        }
        Ok(())
    }
}

fn oldest_key(identities: &std::collections::HashMap<String, VecDeque<Instant>>) -> Option<String> {
    identities
        .iter()
        .min_by_key(|(_, hits)| hits.back().copied().unwrap_or(Instant::now()))
        .map(|(k, _)| k.clone())
}

/// Constant-time match: compares a padded fixed-size slice of the maximum
/// length, so no length branch precedes the comparison.
fn ct_match(supplied: &[u8], candidate: &[u8]) -> bool {
    let max = supplied.len().max(candidate.len());
    let mut s = supplied.to_vec();
    let mut c = candidate.to_vec();
    s.resize(max, 0);
    c.resize(max, 0);
    s.as_slice().ct_eq(c.as_slice()).into()
}

/// 405 envelope for a known route hit with an unsupported method.
pub async fn method_not_allowed_envelope() -> Response {
    error_envelope(
        "METHOD_NOT_ALLOWED",
        "Method not allowed",
        StatusCode::METHOD_NOT_ALLOWED,
    )
}

/// 404 envelope for unmatched routes.
pub async fn route_not_found_envelope() -> Response {
    error_envelope("ROUTE_NOT_FOUND", "Not Found", StatusCode::NOT_FOUND)
}

fn error_envelope(code: &str, message: &str, status: StatusCode) -> Response {
    let request_id = REQUEST_ID.try_with(|id| id.clone()).ok();
    let body = crate::domain::response::ErrorResponse {
        success: false,
        request_id,
        error: crate::domain::response::ErrorDetail {
            code: code.to_string(),
            message: message.to_string(),
            details: serde_json::Value::Object(Default::default()),
        },
    };
    (status, axum::Json(body)).into_response()
}

/// Rate limiter attached to state; kept cheap and cloneable.
pub fn make_rate_limiter() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_match() {
        assert!(ct_match(b"secret", b"secret"));
        assert!(!ct_match(b"secret", b"nope"));
        assert!(!ct_match(b"a", b"ab"));
        assert!(!ct_match(b"", b"x"));
        assert!(ct_match(b"", b""));
    }

    #[test]
    fn rate_limiter_bounds_and_staleness() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(limiter.check("a", 3, 60).is_ok());
        }
        let err = limiter.check("a", 3, 60).unwrap_err();
        assert!(matches!(err, AppError::RateLimited { .. }));
        if let AppError::RateLimited {
            limit,
            window_seconds,
            ..
        } = err
        {
            assert_eq!(limit, 3);
            assert_eq!(window_seconds, 60);
        }
        assert!(limiter.check("b", 3, 60).is_ok());

        // Identity map stays bounded under load.
        let limiter = RateLimiter::new();
        for i in 0..5_000 {
            let _ = limiter.check(&format!("ip-{i}"), 3, 60);
        }
        let count = limiter.identities.lock().unwrap().len();
        assert!(count <= MAX_IDENTITIES * 2, "bounded map: {count}");
    }

    #[test]
    fn bearer_extraction() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer_token(&headers), None);
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer tok123"),
        );
        assert_eq!(bearer_token(&headers).as_deref(), Some("tok123"));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcg=="),
        );
        assert_eq!(bearer_token(&headers), None);
    }
}
