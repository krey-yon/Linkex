//! Upstream error construction and status classification.
//!
//! The one error type is `crate::error::AppError`; this module builds the
//! LinkedIn-facing variants with their stable codes and detail keys.

use serde_json::{Map, Value};

use crate::error::AppError;

pub fn details() -> Map<String, Value> {
    Map::new()
}

pub fn with<M: Into<Map<String, Value>>>(
    endpoint: &str,
    status: Option<i64>,
    extra: M,
) -> Map<String, Value> {
    let mut map = extra.into();
    map.insert("endpoint".to_string(), Value::String(endpoint.to_string()));
    if let Some(status) = status {
        map.insert("status_code".to_string(), Value::from(status));
    }
    map
}

pub fn endpoint_map(endpoint: &str, status: Option<i64>) -> Map<String, Value> {
    with(endpoint, status, details())
}

// ------------------------------------------------------------ classification

/// Statuses that count against the circuit breaker. 404/410 and invalid-user
/// input never count: another strategy can legitimately answer those.
pub fn counts_for_breaker(status: Option<i64>) -> bool {
    matches!(
        status,
        None | Some(429) | Some(999) | Some(500) | Some(502) | Some(503) | Some(504)
    )
}

pub fn auth_failed(message: &str, endpoints: &str) -> AppError {
    AppError::AuthFailed {
        message: message.to_string(),
        details: endpoint_map(endpoints, None),
    }
}

pub fn session_expired(message: &str, endpoint: &str, status: Option<i64>) -> AppError {
    AppError::SessionExpired {
        message: message.to_string(),
        details: endpoint_map(endpoint, status),
    }
}

pub fn profile_not_found(endpoint: &str) -> AppError {
    AppError::ProfileNotFound {
        message: "No LinkedIn profile exists at that URL, or it is not visible to this account."
            .to_string(),
        details: endpoint_map(endpoint, None),
    }
}

pub fn profile_not_visible(identifier: &str, attempted: &[&str]) -> AppError {
    let mut map = details();
    map.insert(
        "identifier".to_string(),
        Value::String(identifier.to_string()),
    );
    map.insert(
        "attempted".to_string(),
        Value::Array(
            attempted
                .iter()
                .map(|s| Value::String(s.to_string()))
                .collect(),
        ),
    );
    AppError::ProfileNotVisible {
        message: "No LinkedIn profile model returned data for this member.".to_string(),
        details: map,
    }
}

pub fn rate_limited(endpoint: &str, status: Option<i64>) -> AppError {
    let mut map = endpoint_map(endpoint, status);
    map.insert("retry_after_seconds".to_string(), Value::from(60));
    AppError::LinkedinRateLimited {
        message: "LinkedIn throttled the request. Back off and retry shortly.".to_string(),
        details: map,
    }
}

pub fn unavailable(endpoint: &str, status: Option<i64>) -> AppError {
    AppError::LinkedinUnavailable {
        message: "LinkedIn returned an unexpected response.".to_string(),
        details: endpoint_map(endpoint, status),
    }
}

pub fn challenge_required(endpoint: &str) -> AppError {
    AppError::ChallengeRequired {
        message: "LinkedIn issued a verification challenge for this account. Complete it in a \
                  browser once, then supply a fresh li_at cookie."
            .to_string(),
        details: endpoint_map(endpoint, None),
    }
}
