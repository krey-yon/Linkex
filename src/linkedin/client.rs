//! Long-lived, session-aware HTTP client for LinkedIn's internal Voyager API.
//!
//! Responsibilities: one connection pool with browser-like headers, exactly
//! one authenticated session (refreshed when LinkedIn rejects it), pacing via
//! the shared throttle and circuit breaker, retry/backoff for idempotent
//! reads, and status-to-error classification. Parsing lives in `parser`;
//! this module never interprets profile content.

use std::sync::Arc;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use rand::Rng;
use reqwest::Client;
use serde_json::Value as JsonValue;
use tokio::sync::Mutex as AsyncMutex;

use crate::config::Settings;

use super::auth::Authenticator;
pub use super::endpoints::VoyagerCall;
use super::error as lerr;
use super::session::{LinkedInSession, SessionStore};
use super::throttle::{CircuitBreaker, RequestThrottle};
use crate::error::AppError;

const RETRYABLE_STATUS: &[i64] = &[429, 500, 502, 503, 504, 999];
const LINKEDIN_THROTTLE_STATUS: &[i64] = &[429, 999];
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// One upstream result, kept alongside the metadata the API surfaces.
#[derive(Debug, Clone)]
pub struct VoyagerResponse {
    pub name: &'static str,
    pub status_code: i64,
    pub payload: Option<JsonValue>,
    pub elapsed_ms: i64,
    pub attempts: i64,
}

impl VoyagerResponse {
    pub fn ok(&self) -> bool {
        self.status_code == 200 && self.payload.is_some()
    }
}

/// Executor seam for the repository: production client and mock executors
/// (tests) substitute. Object-safe via pinned futures borrowing `&self`.
pub trait Upstream: Send + Sync + 'static {
    fn fetch(
        &self,
        call: VoyagerCall,
        referer: String,
        allow_fallback: bool,
    ) -> BoxFuture<'_, Result<VoyagerResponse, AppError>>;
}

/// Non-secret diagnostics for `/readyz` and `/v1/session`.
pub trait SessionDiagnostics: Send + Sync + 'static {
    fn ensure_session(&self) -> BoxFuture<'_, Result<(), AppError>>;
    fn state(&self) -> JsonValue;
}

pub struct VoyagerClient {
    settings: Arc<Settings>,
    auth: Authenticator,
    store: Arc<dyn SessionStore>,
    throttle: RequestThrottle,
    breaker: CircuitBreaker,
    client: RwLock<Option<Client>>,
    jar: Arc<reqwest_cookie_store::CookieStoreMutex>,
    session: RwLock<Option<LinkedInSession>>,
    auth_lock: AsyncMutex<()>,
    last_save: Mutex<Instant>,
}

impl VoyagerClient {
    pub fn new(settings: Arc<Settings>, store: Arc<dyn SessionStore>) -> Self {
        let jar = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            reqwest_cookie_store::CookieStore::default(),
        ));
        let auth = Authenticator::new(settings.clone());
        VoyagerClient {
            settings: settings.clone(),
            auth,
            store,
            throttle: RequestThrottle::new(
                settings.upstream_min_interval_seconds,
                settings.upstream_jitter_seconds,
                settings.upstream_max_concurrency,
            ),
            breaker: CircuitBreaker::new(
                settings.circuit_breaker_threshold,
                settings.circuit_breaker_cooldown_seconds,
            ),
            client: RwLock::new(None),
            jar,
            session: RwLock::new(None),
            auth_lock: AsyncMutex::new(()),
            last_save: Mutex::new(Instant::now()),
        }
    }

    pub fn breaker(&self) -> &CircuitBreaker {
        &self.breaker
    }

    // ----------------------------------------------------------------- lifecycle

    pub fn ensure_client(&self) -> Client {
        if let Some(client) = self.client.read().unwrap().as_ref() {
            return client.clone();
        }
        let mut guard = self.client.write().unwrap();
        if let Some(client) = guard.as_ref() {
            return client.clone();
        }
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.settings.request_timeout)
            .pool_max_idle_per_host(5)
            .pool_idle_timeout(Duration::from_secs(90))
            .cookie_provider(self.jar.clone());
        if let Some(proxy) = self.settings.proxy_url()
            && let Ok(proxy) = reqwest::Proxy::all(proxy)
        {
            builder = builder.proxy(proxy);
        }
        let client = builder.build().expect("reqwest client construction");
        tracing::info!("voyager.client_started");
        *guard = Some(client.clone());
        client
    }

    pub fn is_authenticated(&self) -> bool {
        self.session
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.is_usable())
    }

    /// Redacted diagnostics for `/readyz` and `/v1/session`.
    pub fn diagnostics(&self) -> JsonValue {
        let session_state = {
            let session = self.session.read().unwrap();
            match session.as_ref() {
                Some(session) => session.public_state(),
                None => JsonValue::Null,
            }
        };
        serde_json::json!({
            "session": if session_state.is_null() {
                serde_json::json!({"authenticated": false, "source": JsonValue::Null})
            } else {
                session_state
            },
            "circuit": self.breaker.snapshot(),
        })
    }

    pub fn session_snapshot(&self) -> Option<LinkedInSession> {
        self.session.read().unwrap().clone()
    }

    /// Return the active session, authenticating at most once concurrently:
    /// the auth mutex spans the whole refresh and state is re-checked inside.
    pub async fn ensure_session(&self, force: bool) -> Result<LinkedInSession, AppError> {
        let _guard = self.auth_lock.lock().await;
        {
            let session = self.session.read().unwrap();
            if !force
                && let Some(session) = session.as_ref()
                && session.is_usable()
            {
                return Ok(session.clone());
            }
        }

        let client = self.ensure_client();
        let session = self
            .auth
            .authenticate(&client, &self.jar, &self.store)
            .await?;
        self.auth.apply_session(&self.jar, &session);
        {
            let mut current = self.session.write().unwrap();
            *current = Some(session.clone());
        }
        {
            let store = self.store.clone();
            let session_clone = session.clone();
            let _ = tokio::task::spawn_blocking(move || store.save(&session_clone)).await;
        }
        Ok(session)
    }

    /// Persist rotated cookies at a controlled frequency (at most every two
    /// minutes) so a restart does not re-authenticate with stale cookies.
    async fn persist_if_due(&self) {
        let save_now = {
            let mut last = self.last_save.lock().unwrap();
            if last.elapsed() >= Duration::from_secs(120) {
                *last = Instant::now();
                true
            } else {
                false
            }
        };
        if save_now {
            let store = self.store.clone();
            let session = self.session.read().unwrap().clone();
            if let Some(session) = session
                && session.is_usable()
            {
                tracing::info!("session.persisted_after_rotation");
                let _ = tokio::task::spawn_blocking(move || store.save(&session)).await;
            }
        }
    }

    // ------------------------------------------------------------------ fetching

    /// Execute *call*, retrying transient failures. `allow_fallback` marks a
    /// call another strategy may replace: soft failures return a non-OK
    /// `VoyagerResponse`, while definitive answers (404, throttling, dead
    /// session) still raise.
    pub async fn fetch(
        &self,
        call: VoyagerCall,
        referer: String,
        allow_fallback: bool,
    ) -> Result<VoyagerResponse, AppError> {
        self.breaker.check()?;
        let session = self.ensure_session(false).await?;

        let required = call.required && !allow_fallback;
        let mut last_error: Option<AppError> = None;
        let mut reauthenticated = false;
        let attempts = self.settings.max_retries.max(1);

        for attempt in 1..=attempts {
            let _permit = self.throttle.acquire().await;
            let (status, payload, elapsed_ms) = self.send(&call, &session, &referer).await;

            if status == 200 {
                if let Some(payload) = payload {
                    self.breaker.record_success();
                    return Ok(VoyagerResponse {
                        name: call.name,
                        status_code: 200,
                        payload: Some(payload),
                        elapsed_ms,
                        attempts: attempt as i64,
                    });
                }
                last_error = Some(lerr::unavailable(call.name, Some(200)));
            } else if is_redirect(status) {
                tracing::warn!(endpoint = call.name, "voyager.redirected");
                if session.signed_out() {
                    return Err(lerr::session_expired(
                        "LinkedIn signed this session out mid-request. Capture a fresh \
                         cookie set from a signed-in browser (the whole Cookie header).",
                        call.name,
                        Some(status),
                    ));
                }
                last_error = Some(lerr::unavailable(call.name, Some(status)));
                break;
            } else if status == 401 || status == 403 {
                // One re-auth per fetch regardless of fallback: a stale
                // session must self-heal instead of silently degrading a
                // profile request to PROFILE_NOT_VISIBLE.
                if !reauthenticated {
                    reauthenticated = true;
                    tracing::warn!(
                        endpoint = call.name,
                        status = status,
                        "voyager.reauthenticating"
                    );
                    match self.ensure_session(true).await {
                        Ok(fresh) => {
                            {
                                let mut current = self.session.write().unwrap();
                                *current = Some(fresh.clone());
                            }
                            continue;
                        }
                        Err(e) => {
                            self.breaker.record_failure();
                            return Err(e);
                        }
                    }
                }
                self.breaker.record_failure();
                let err = lerr::session_expired(
                    "The LinkedIn session cookie is no longer valid; re-seed credentials.",
                    call.name,
                    Some(status),
                );
                if required {
                    return Err(err);
                }
                return Ok(VoyagerResponse {
                    name: call.name,
                    status_code: status,
                    payload: None,
                    elapsed_ms,
                    attempts: attempt as i64,
                });
            } else if status == 404 {
                if call.required || allow_fallback {
                    return Err(lerr::profile_not_found(call.name));
                }
                return Ok(VoyagerResponse {
                    name: call.name,
                    status_code: status,
                    payload: None,
                    elapsed_ms,
                    attempts: attempt as i64,
                });
            } else if status == 410 {
                // LinkedIn retired the endpoint: never retry, move on.
                tracing::warn!(endpoint = call.name, "voyager.endpoint_retired");
                return Ok(VoyagerResponse {
                    name: call.name,
                    status_code: status,
                    payload: None,
                    elapsed_ms,
                    attempts: attempt as i64,
                });
            } else if LINKEDIN_THROTTLE_STATUS.contains(&status) {
                last_error = Some(lerr::rate_limited(call.name, Some(status)));
                self.breaker.record_failure();
            } else if RETRYABLE_STATUS.contains(&status) {
                last_error = Some(lerr::unavailable(call.name, Some(status)));
            } else {
                last_error = Some(lerr::unavailable(call.name, Some(status)));
                break;
            }

            if attempt < attempts {
                tokio::time::sleep(self.backoff(attempt)).await;
            }
        }

        self.breaker.record_failure();
        if required {
            return Err(last_error.unwrap_or_else(|| lerr::unavailable(call.name, None)));
        }
        tracing::warn!(
            endpoint = call.name,
            error = last_error
                .as_ref()
                .map(|e| e.code().to_string())
                .unwrap_or_default(),
            "voyager.optional_call_failed"
        );
        Ok(VoyagerResponse {
            name: call.name,
            status_code: 0,
            payload: None,
            elapsed_ms: 0,
            attempts: attempts as i64,
        })
    }

    /// One HTTP round trip: headers, send, decode JSON, absorb rotated
    /// cookies. Returns (status, decoded payload, elapsed milliseconds).
    async fn send(
        &self,
        call: &VoyagerCall,
        session: &LinkedInSession,
        referer: &str,
    ) -> (i64, Option<JsonValue>, i64) {
        let client = self.ensure_client();
        let started = Instant::now();
        let mut headers = self.auth.voyager_headers(session, referer);
        headers.insert(
            "x-li-page-instance",
            reqwest::header::HeaderValue::from_str(&self.auth.li_page_instance())
                .expect("header value from urn"),
        );
        if call.normalized {
            headers.insert(reqwest::header::ACCEPT, self.auth.normalized_accept());
        }

        let url = format!("{}{}", super::endpoints::BASE_URL, call.path);
        let result = client
            .get(&url)
            .query(&call.params)
            .headers(headers)
            .send()
            .await;

        let response = match result {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(endpoint = call.name, error = %err, "voyager.transport_error");
                return (0, None, started.elapsed().as_millis() as i64);
            }
        };
        let elapsed = started.elapsed().as_millis() as i64;
        let status = response.status().as_u16() as i64;

        // Absorb rotated cookies so the session ages like a browser session.
        {
            let mut session_copy = session.clone();
            self.auth.absorb_cookies(&self.jar, &mut session_copy);
            if !session_copy
                .cookies
                .iter()
                .all(|(k, v)| session.cookies.get(k).is_some_and(|existing| existing == v))
            {
                let mut current = self.session.write().unwrap();
                *current = Some(session_copy);
            }
        }
        self.persist_if_due().await;

        tracing::info!(
            endpoint = call.name,
            status = status,
            elapsed_ms = elapsed,
            "voyager.request"
        );

        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            let body = &body[..body.len().min(4096)];
            tracing::warn!(
                endpoint = call.name,
                status,
                body = %body,
                "voyager.request_failed"
            );
            return (status, None, elapsed);
        }
        let payload = decode_json(response).await;
        (status, payload, elapsed)
    }

    fn backoff(&self, attempt: usize) -> Duration {
        let base = self.settings.retry_backoff_seconds * f64::powi(2.0, (attempt - 1) as i32);
        let jitter = rand::rng().random_range(0.0..self.settings.retry_backoff_seconds);
        Duration::from_secs_f64(base + jitter)
    }

    /// Validate the session against `/voyager/api/me` without caching the
    /// outcome — used by `/readyz`.
    pub async fn ensure_ready(&self) -> Result<(), AppError> {
        self.ensure_session(false).await.map(|_| ())
    }
}

fn is_redirect(status: i64) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

async fn decode_json(response: reqwest::Response) -> Option<JsonValue> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("json") {
        return None;
    }
    let text = match response.text().await {
        Ok(text) => text,
        Err(_) => return None,
    };
    if text.len() > MAX_BODY_BYTES {
        tracing::warn!(bytes = text.len(), "voyager.body_above_cap");
        return None;
    }
    match serde_json::from_str::<JsonValue>(&text) {
        Ok(payload @ JsonValue::Object(_)) => Some(payload),
        _ => None,
    }
}

impl Upstream for VoyagerClient {
    fn fetch(
        &self,
        call: VoyagerCall,
        referer: String,
        allow_fallback: bool,
    ) -> BoxFuture<'_, Result<VoyagerResponse, AppError>> {
        Box::pin(async move { self.fetch(call, referer, allow_fallback).await })
    }
}

impl SessionDiagnostics for VoyagerClient {
    fn ensure_session(&self) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move { self.ensure_session(false).await.map(|_| ()) })
    }

    fn state(&self) -> JsonValue {
        self.diagnostics()
    }
}
