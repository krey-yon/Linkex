//! Composition root: builds the Axum router from an `AppState`, wiring
//! middleware in a deliberate order and routing to the API handlers.

use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{MethodRouter, any, get};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;

use crate::api::account;
use crate::api::middleware::{
    api_key_and_rate_limit, method_not_allowed_envelope, request_id_middleware,
};
use crate::api::profile;
use crate::api::system;
use crate::linkedin::client::Upstream;
use crate::state::AppState;

/// Build the fully composed application router.
///
/// Layer order (outermost first): CORS → body limit → request id (task-local,
/// response headers) → route handlers, with API-key auth + rate limiting on
/// the `/v1` routes themselves.
pub fn build_app<U: Upstream>(state: AppState<U>) -> Router
where
    AppState<U>: std::marker::Send + std::marker::Sync + 'static,
{
    let settings = state.settings.clone();
    let cors = cors_layer(&settings.cors_origins);

    let protected = Router::new()
        .route(
            "/v1/profile",
            MethodRouter::new()
                .get(profile::get_profile::<U>)
                .post(profile::post_profile::<U>)
                .fallback(method_not_allowed_envelope),
        )
        .route(
            "/v1/profile/raw",
            get(profile::get_profile_raw::<U>).fallback(method_not_allowed_envelope),
        )
        .route(
            "/v1/account",
            get(account::get_account::<U>).fallback(method_not_allowed_envelope),
        )
        .route(
            "/v1/session",
            get(system::session_state::<U>).fallback(method_not_allowed_envelope),
        )
        .route_layer(middleware::from_fn_with_state(
            Arc::new(state.clone()),
            api_key_and_rate_limit::<U>,
        ));

    let open = Router::new()
        .route("/healthz", get(system::healthz::<U>))
        .route("/readyz", get(system::readyz::<U>));

    Router::new()
        .merge(protected)
        .merge(open)
        .with_state(Arc::new(state))
        .layer(middleware::from_fn(request_id_middleware))
        .layer(RequestBodyLimitLayer::new(settings.max_request_body_bytes))
        .layer(cors)
        // Static site: `/` serves site/index.html (ServeDir appends index.html
        // for directory paths); the JSON envelope stays the not-found fallback.
        .fallback_service(
            ServeDir::new("site")
                .not_found_service(any(crate::api::middleware::route_not_found_envelope)),
        )
}

fn cors_layer(origins: &[String]) -> CorsLayer {
    use tower_http::cors::{AllowOrigin, Any};

    let layer = CorsLayer::new()
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::HeaderName::from_static("x-api-key"),
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ])
        .expose_headers([
            axum::http::header::HeaderName::from_static("x-request-id"),
            axum::http::header::HeaderName::from_static("x-cache"),
            axum::http::header::HeaderName::from_static("x-response-time-ms"),
            axum::http::header::HeaderName::from_static("x-credit-balance-cents"),
            axum::http::header::HeaderName::from_static("x-request-cost-cents"),
        ]);
    if origins.iter().any(|o| o == "*") {
        layer.allow_origin(Any)
    } else {
        let headers: Vec<axum::http::HeaderValue> = origins
            .iter()
            .filter_map(|o| axum::http::HeaderValue::from_str(o).ok())
            .collect();
        layer.allow_origin(AllowOrigin::list(headers))
    }
}
