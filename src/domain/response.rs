//! Envelopes shared by every endpoint, matching the Python contract exactly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::profile::Profile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileRequest {
    pub url: String,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseMeta {
    pub request_id: String,
    #[serde(default)]
    pub cached: bool,
    pub cache_age_seconds: Option<f64>,
    pub elapsed_ms: i64,
    #[serde(default)]
    pub upstream_calls: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileResponse {
    #[serde(default = "default_true")]
    pub success: bool,
    pub meta: ResponseMeta,
    pub data: Profile,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    #[serde(default)]
    pub success: bool,
    pub request_id: Option<String>,
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub environment: String,
    pub uptime_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionResponse {
    pub session: Value,
    pub circuit: Value,
    pub cache: Value,
    pub checked_at: DateTime<Utc>,
}
