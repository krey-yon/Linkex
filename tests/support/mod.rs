//! Shared helpers for integration tests: in-memory upstream, app/router
//! builders, and fixture loading. Everything here runs offline.
//!
//! Each integration test compiles this module separately, so no single test
//! uses every helper; the allow keeps partially-used items from failing a
//! `-D warnings` Clippy run.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::BoxFuture;
use serde_json::Value;

use tross::api::middleware::make_rate_limiter;
use tross::app::build_app;
use tross::billing::BillingStore;
use tross::config::Settings;
use tross::error::AppError;
use tross::linkedin::client::{Upstream, VoyagerCall, VoyagerResponse};
use tross::linkedin::repository::ProfileRepository;
use tross::linkedin::session::InMemoryStore;
use tross::service::cache::ProfileCache;
use tross::service::profile::ProfileService;
use tross::state::AppState;

pub const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");

pub fn fixture(name: &str) -> Value {
    let raw = std::fs::read_to_string(format!("{FIXTURES}/{name}"))
        .unwrap_or_else(|e| panic!("fixture {name}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("fixture {name} invalid JSON: {e}"))
}

type Canned = Vec<(String, (i64, Option<Value>))>;

/// Mock upstream that answers with canned Voyager responses, recording every
/// call so tests can assert counts. Strategy names (dashProfile, etc.) are
/// counted separately from enrichment calls.
pub struct MockUpstream {
    calls: AtomicUsize,
    profile_calls: AtomicUsize,
    responses: std::sync::Mutex<Canned>,
}

impl MockUpstream {
    pub fn new() -> Arc<Self> {
        Arc::new(MockUpstream {
            calls: AtomicUsize::new(0),
            profile_calls: AtomicUsize::new(0),
            responses: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Configure the canned answer for one endpoint name (e.g. "dashProfile").
    /// Unanswered endpoints return 404.
    pub fn respond(&self, name: &str, status: i64, payload: Option<Value>) {
        self.responses
            .lock()
            .unwrap()
            .push((name.to_string(), (status, payload)));
    }

    pub fn total_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn strategy_calls(&self) -> usize {
        self.profile_calls.load(Ordering::SeqCst)
    }
}

impl Upstream for MockUpstream {
    fn fetch(
        &self,
        call: VoyagerCall,
        _referer: String,
        _allow_fallback: bool,
    ) -> BoxFuture<'_, Result<VoyagerResponse, AppError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let responses = self.responses.lock().unwrap().clone();
        let resolved = responses
            .iter()
            .find(|(name, _)| name == call.name)
            .map(|(_, (status, payload))| (*status, payload.clone()));
        let is_strategy = matches!(
            call.name,
            "dashProfile" | "dashProfileMinimal" | "graphqlProfile" | "profileView"
        );

        Box::pin(async move {
            if is_strategy {
                self.profile_calls.fetch_add(1, Ordering::SeqCst);
            }
            let (status, payload) = resolved.unwrap_or((404, None));
            Ok(VoyagerResponse {
                name: call.name,
                status_code: status,
                payload,
                elapsed_ms: 5,
                attempts: 1,
            })
        })
    }
}

/// Build the fully composed app around a fresh mock upstream.
pub fn build_app_with(settings: Settings, upstream: Arc<MockUpstream>) -> axum::Router {
    let state = make_state(settings, upstream);
    build_app::<MockUpstream>(state)
}

pub fn build_app_default() -> (axum::Router, Arc<MockUpstream>) {
    let upstream = MockUpstream::new();
    let router = build_app_with(Settings::default(), upstream.clone());
    (router, upstream)
}

pub fn make_state(settings: Settings, upstream: Arc<MockUpstream>) -> AppState<MockUpstream> {
    let settings = Arc::new(settings);
    let repo = Arc::new(ProfileRepository::new(upstream, settings.clone()));
    let cache = Arc::new(ProfileCache::new(900, 32, None, false));
    let service = Arc::new(ProfileService::new(repo, cache));
    AppState {
        settings: settings.clone(),
        service,
        voyager: Arc::new(tross::linkedin::client::VoyagerClient::new(
            settings.clone(),
            Arc::new(InMemoryStore::shared()),
        )),
        billing: BillingStore::Disabled,
        rate_limiter: make_rate_limiter(),
        started_at: std::time::Instant::now(),
    }
}
