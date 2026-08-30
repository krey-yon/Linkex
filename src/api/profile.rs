//! Profile lookup endpoints — the core of the service. Handlers only extract
//! and validate request data, call the service, set cache headers, and map
//! typed output.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::json;

use crate::domain::response::{ProfileResponse, ResponseMeta};
use crate::error::{AppError, REQUEST_ID};
use crate::linkedin::client::Upstream;
use crate::linkedin::url::parse_profile_url;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ProfileQuery {
    pub url: String,
    #[serde(default)]
    pub refresh: bool,
}

pub async fn get_profile<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
    Query(query): Query<ProfileQuery>,
) -> Result<(HeaderMap, Json<ProfileResponse>), AppError> {
    lookup(state, query.url, query.refresh).await
}

pub async fn post_profile<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
    payload: StrictJson<crate::domain::response::ProfileRequest>,
) -> Result<(HeaderMap, Json<ProfileResponse>), AppError> {
    lookup(state, payload.0.url, payload.0.refresh).await
}

/// Extractor like `Json`, but a malformed body or an unknown field is a
/// VALIDATION_ERROR (422), matching the Python contract's `extra="forbid"`.
pub struct StrictJson<T>(pub T);

impl<S, T> axum::extract::FromRequest<S> for StrictJson<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Json(inner) = axum::extract::Json::<T>::from_request(req, state)
            .await
            .map_err(|rejection| {
            tracing::warn!(reason = %rejection.body_text(), "api.json_rejection");
            AppError::Validation {
                message: "The request body is not valid JSON.".to_string(),
                field: None,
            }
        })?;
        Ok(StrictJson(inner))
    }
}

async fn lookup<U: Upstream>(
    state: std::sync::Arc<AppState<U>>,
    url: String,
    refresh: bool,
) -> Result<(HeaderMap, Json<ProfileResponse>), AppError> {
    let result = state.service.get_profile(&url, refresh).await?;
    let request_id = REQUEST_ID.try_with(|id| id.clone()).unwrap_or_default();

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-cache",
        (if result.cached { "HIT" } else { "MISS" })
            .parse()
            .expect("static"),
    );
    headers.insert(
        "cache-control",
        "private, no-store".parse().expect("static"),
    );

    Ok((
        headers,
        Json(ProfileResponse {
            success: true,
            meta: ResponseMeta {
                request_id,
                cached: result.cached,
                cache_age_seconds: result.cache_age_seconds,
                elapsed_ms: result.elapsed_ms,
                upstream_calls: result.upstream_calls,
            },
            data: (*result.profile).clone(),
        }),
    ))
}

/// Raw upstream payloads for one profile (diagnostics; disabled by default).
/// The payloads are large and undocumented — documentation/operational
/// warning: only enable while debugging.
pub async fn get_profile_raw<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
    Query(query): Query<ProfileQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::linkedin::endpoints;

    if !state.settings.expose_raw_endpoint {
        return Err(AppError::EndpointDisabled);
    }
    let ref_ = parse_profile_url(&query.url)?;
    let calls = [
        endpoints::profile_view(&ref_.identifier),
        endpoints::profile_core(&ref_.identifier),
        endpoints::profile_contact_info(&ref_.identifier),
        endpoints::profile_skills(&ref_.identifier, 100),
        endpoints::profile_network_info(&ref_.identifier),
    ];
    let mut payloads = serde_json::Map::new();
    for call in calls {
        let name = call.name;
        let result = state
            .service
            .raw_fetch(call, ref_.canonical_url.clone())
            .await;
        match result {
            Ok(response) => {
                payloads.insert(
                    name.to_string(),
                    json!({"status_code": response.status_code, "payload": response.payload}),
                );
            }
            Err(err) => {
                payloads.insert(
                    name.to_string(),
                    json!({"status_code": 0, "error": err.code()}),
                );
            }
        }
    }
    Ok(Json(
        json!({"identifier": ref_.identifier, "calls": payloads}),
    ))
}
