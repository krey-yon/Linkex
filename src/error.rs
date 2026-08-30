//! Typed error taxonomy and HTTP mapping (Axum `IntoResponse`).
//!
//! Every failure leaves through one envelope shape:
//! `{ "success": false, "request_id": ..., "error": { code, message, details } }`
//! with stable machine-readable codes. Secrets never appear in messages,
//! details, or logged error chains.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use crate::domain::response::{ErrorDetail, ErrorResponse};

tokio::task_local! {
    /// The current request id, set by the request-id middleware.
    pub static REQUEST_ID: String;
}

fn details(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// Upstream/domain errors carry the same shape as the Python `LinkedInError`
/// hierarchy: a fixed message, and free-form detail keys (endpoint, status
/// code, retry-after) that degrade to nothing when absent.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("validation error: {message}")]
    Validation {
        message: String,
        field: Option<String>,
    },
    #[error("invalid profile url: {message}")]
    InvalidProfileUrl {
        message: String,
        details: Map<String, Value>,
    },
    #[error("api key missing")]
    ApiKeyMissing,
    #[error("api key invalid")]
    ApiKeyInvalid,
    #[error("insufficient credit")]
    InsufficientCredit {
        balance_cents: i64,
        required_cents: i64,
    },
    #[error("billing unavailable")]
    BillingUnavailable,
    #[error("rate limited")]
    RateLimited {
        retry_after_seconds: f64,
        limit: u64,
        window_seconds: u64,
    },
    #[error("linkedin auth failed: {message}")]
    AuthFailed {
        message: String,
        details: Map<String, Value>,
    },
    #[error("linkedin session expired: {message}")]
    SessionExpired {
        message: String,
        details: Map<String, Value>,
    },
    #[error("linkedin challenge required: {message}")]
    ChallengeRequired {
        message: String,
        details: Map<String, Value>,
    },
    #[error("profile not found: {message}")]
    ProfileNotFound {
        message: String,
        details: Map<String, Value>,
    },
    #[error("profile not visible: {message}")]
    ProfileNotVisible {
        message: String,
        details: Map<String, Value>,
    },
    #[error("linkedin rate limited: {message}")]
    LinkedinRateLimited {
        message: String,
        details: Map<String, Value>,
    },
    #[error("linkedin unavailable: {message}")]
    LinkedinUnavailable {
        message: String,
        details: Map<String, Value>,
    },
    #[error("upstream circuit open: {message}")]
    CircuitOpen {
        message: String,
        details: Map<String, Value>,
    },
    #[error("endpoint disabled")]
    EndpointDisabled,
    #[error("internal error: {context}")]
    Internal { context: String },
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation { .. } => "VALIDATION_ERROR",
            Self::InvalidProfileUrl { .. } => "INVALID_PROFILE_URL",
            Self::ApiKeyMissing => "API_KEY_MISSING",
            Self::ApiKeyInvalid => "API_KEY_INVALID",
            Self::InsufficientCredit { .. } => "INSUFFICIENT_CREDIT",
            Self::BillingUnavailable => "BILLING_UNAVAILABLE",
            Self::RateLimited { .. } => "RATE_LIMITED",
            Self::AuthFailed { .. } => "LINKEDIN_AUTH_FAILED",
            Self::SessionExpired { .. } => "LINKEDIN_SESSION_EXPIRED",
            Self::ChallengeRequired { .. } => "LINKEDIN_CHALLENGE_REQUIRED",
            Self::ProfileNotFound { .. } => "PROFILE_NOT_FOUND",
            Self::ProfileNotVisible { .. } => "PROFILE_NOT_VISIBLE",
            Self::LinkedinRateLimited { .. } => "LINKEDIN_RATE_LIMITED",
            Self::LinkedinUnavailable { .. } => "LINKEDIN_UNAVAILABLE",
            Self::CircuitOpen { .. } => "UPSTREAM_CIRCUIT_OPEN",
            Self::EndpointDisabled => "ENDPOINT_DISABLED",
            Self::Internal { .. } => "INTERNAL_ERROR",
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Validation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::InvalidProfileUrl { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::ApiKeyMissing => StatusCode::UNAUTHORIZED,
            Self::ApiKeyInvalid => StatusCode::FORBIDDEN,
            Self::InsufficientCredit { .. } => StatusCode::PAYMENT_REQUIRED,
            Self::BillingUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::AuthFailed { .. }
            | Self::SessionExpired { .. }
            | Self::ChallengeRequired { .. }
            | Self::CircuitOpen { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::ProfileNotFound { .. } => StatusCode::NOT_FOUND,
            Self::ProfileNotVisible { .. } => StatusCode::FORBIDDEN,
            Self::LinkedinRateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::LinkedinUnavailable { .. } => StatusCode::BAD_GATEWAY,
            Self::EndpointDisabled => StatusCode::NOT_FOUND,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn retry_after_seconds(&self) -> Option<i64> {
        let value = match self {
            Self::RateLimited {
                retry_after_seconds,
                ..
            } => Some(*retry_after_seconds),
            Self::LinkedinRateLimited { details, .. } | Self::CircuitOpen { details, .. } => {
                details
                    .get("retry_after_seconds")
                    .and_then(Value::as_f64)
                    .or_else(|| {
                        details
                            .get("retry_after_seconds")
                            .and_then(Value::as_i64)
                            .map(|v| v as f64)
                    })
            }
            _ => None,
        };
        value.map(|v| v.round().max(0.0) as i64)
    }

    fn detail_value(&self) -> Value {
        let empty = Map::new();
        let map = match self {
            Self::Validation { field, .. } => {
                let mut m = Map::new();
                if let Some(field) = field {
                    m.insert("field".into(), Value::String(field.clone()));
                } else {
                    m.insert("field".into(), Value::Null);
                }
                m
            }
            Self::InvalidProfileUrl { details, .. }
            | Self::AuthFailed { details, .. }
            | Self::SessionExpired { details, .. }
            | Self::ChallengeRequired { details, .. }
            | Self::ProfileNotFound { details, .. }
            | Self::ProfileNotVisible { details, .. }
            | Self::LinkedinRateLimited { details, .. }
            | Self::LinkedinUnavailable { details, .. }
            | Self::CircuitOpen { details, .. } => details.clone(),
            Self::InsufficientCredit {
                balance_cents,
                required_cents,
            } => {
                let mut m = Map::new();
                m.insert("balance_cents".into(), (*balance_cents).into());
                m.insert("required_cents".into(), (*required_cents).into());
                m
            }
            Self::RateLimited {
                limit,
                window_seconds,
                ..
            } => {
                let mut m = Map::new();
                m.insert("limit".into(), (*limit as i64).into());
                m.insert("window_seconds".into(), (*window_seconds as i64).into());
                m
            }
            _ => empty,
        };
        Value::Object(map)
    }

    fn display_message(&self) -> String {
        match self {
            Self::RateLimited {
                retry_after_seconds,
                ..
            } => {
                format!(
                    "Too many requests. Retry in {}s.",
                    retry_after_seconds.round() as i64
                )
            }
            Self::Validation { message, .. } => message.clone(),
            Self::InvalidProfileUrl { message, .. } => message.clone(),
            Self::ApiKeyMissing => "Provide your API key in the X-API-Key header.".to_string(),
            Self::ApiKeyInvalid => "The supplied API key is not valid.".to_string(),
            Self::InsufficientCredit { .. } => {
                "This API key does not have enough credit for the request.".to_string()
            }
            Self::BillingUnavailable => "Credit accounting is temporarily unavailable.".to_string(),
            Self::AuthFailed { message, .. }
            | Self::SessionExpired { message, .. }
            | Self::ChallengeRequired { message, .. }
            | Self::ProfileNotFound { message, .. }
            | Self::ProfileNotVisible { message, .. }
            | Self::LinkedinRateLimited { message, .. }
            | Self::LinkedinUnavailable { message, .. }
            | Self::CircuitOpen { message, .. } => message.clone(),
            Self::EndpointDisabled => {
                "Set EXPOSE_RAW_ENDPOINT=true to enable the raw diagnostics endpoint.".to_string()
            }
            Self::Internal { .. } => {
                "An unexpected error occurred. Quote the request id when reporting it.".to_string()
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let code = self.code();

        match &self {
            Self::Internal { context } => {
                tracing::error!(code, status = ?status, context = %context, "internal error");
            }
            _ => tracing::warn!(code, status = ?status, "api.error"),
        }

        let request_id = REQUEST_ID.try_with(|id| id.clone()).ok();
        let retry_after = self.retry_after_seconds();

        let body = ErrorResponse {
            success: false,
            request_id,
            error: ErrorDetail {
                code: code.to_string(),
                message: self.display_message(),
                details: self.detail_value(),
            },
        };

        let mut response = (status, Json(body)).into_response();
        if let Some(retry_after) = retry_after {
            let value = axum::http::HeaderValue::from_str(&retry_after.to_string())
                .expect("header value from small integer");
            response.headers_mut().insert("Retry-After", value);
        }
        response
    }
}

pub fn details_from(endpoint: &str, status: Option<i64>) -> Map<String, Value> {
    details(&[
        ("endpoint", Value::String(endpoint.to_string())),
        (
            "status_code",
            status.map(Value::from).unwrap_or(Value::Null),
        ),
    ])
}
