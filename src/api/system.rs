//! Liveness, readiness, and upstream session diagnostics. `/healthz` never
//! touches LinkedIn; `/readyz` may validate the session and must return 503
//! when unavailable. `/` serves the static site via ServeDir (see app.rs).

use axum::Json;
use axum::extract::State;
use chrono::Utc;
use serde_json::json;

use crate::domain::response::{HealthResponse, SessionResponse};
use crate::linkedin::client::Upstream;
use crate::state::AppState;

pub async fn healthz<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
) -> Json<HealthResponse> {
    let uptime = state.started_at.elapsed().as_secs_f64();
    Json(HealthResponse {
        status: "ok".to_string(),
        version: state.settings.app_version.clone(),
        environment: state.settings.environment.as_str().to_string(),
        uptime_seconds: (uptime * 10.0).round() / 10.0,
    })
}

pub async fn readyz<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let checked_at = Utc::now();
    match state.voyager.ensure_session().await {
        Ok(()) => (
            axum::http::StatusCode::OK,
            Json(json!({
                "ready": true,
                "upstream": state.voyager.state(),
                "error": null,
                "checked_at": checked_at,
            })),
        ),
        Err(err) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ready": false,
                "upstream": state.voyager.state(),
                "error": {"code": err.code(), "message": err.to_string()},
                "checked_at": checked_at,
            })),
        ),
    }
}

pub async fn session_state<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
) -> Json<SessionResponse> {
    let diagnostics = state.voyager.state();
    Json(SessionResponse {
        session: diagnostics
            .get("session")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        circuit: diagnostics
            .get("circuit")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        cache: state.service.cache_stats(),
        checked_at: Utc::now(),
    })
}
