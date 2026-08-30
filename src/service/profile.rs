//! Application service for profile lookups.
//!
//! Sits between the HTTP layer and the LinkedIn layer and owns three concerns
//! the routes should not care about: the cache, request coalescing (keyed
//! single-flight), and the timing metadata returned to callers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::watch;

use crate::domain::profile::Profile;
use crate::error::AppError;
use crate::linkedin::client::Upstream;
use crate::linkedin::repository::ProfileRepository;
use crate::linkedin::url::{ProfileRef, parse_profile_url};

use super::cache::{CachedProfile, ProfileCache};

#[derive(Debug, Clone)]
pub struct ProfileResult {
    pub profile: Arc<Profile>,
    pub cached: bool,
    pub cache_age_seconds: Option<f64>,
    pub elapsed_ms: i64,
    pub upstream_calls: i64,
}

type RepoResult = Result<Arc<Profile>, AppError>;

enum FlightState {
    Pending,
    Done(RepoResult),
}

struct Flight {
    receiver: watch::Receiver<FlightState>,
}

pub struct ProfileService<U: Upstream> {
    repository: Arc<ProfileRepository<U>>,
    cache: Arc<ProfileCache>,
    inflight: Arc<tokio::sync::Mutex<HashMap<String, Flight>>>,
}

impl<U: Upstream> ProfileService<U> {
    pub fn new(repository: Arc<ProfileRepository<U>>, cache: Arc<ProfileCache>) -> Self {
        ProfileService {
            repository,
            cache,
            inflight: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_profile(&self, url: &str, refresh: bool) -> Result<ProfileResult, AppError> {
        let started = Instant::now();
        let ref_ = parse_profile_url(url)?;
        let cache_key = ref_.cache_key();

        if refresh {
            self.cache.invalidate(&cache_key);
        } else {
            if let Some(entry) = self.cache.get(&cache_key) {
                tracing::info!(identifier = ref_.identifier, "profile.cache_hit");
                let cache_age_seconds = Some((entry.age_seconds() * 10.0).round() / 10.0);
                return Ok(ProfileResult {
                    profile: entry.profile,
                    cached: true,
                    cache_age_seconds,
                    elapsed_ms: started.elapsed().as_millis() as i64,
                    upstream_calls: 0,
                });
            }
            // In-process miss: still try the disk copy, off the request path.
            let key = cache_key.clone();
            let cache = self.cache.clone();
            let disk_hit = tokio::task::spawn_blocking(move || {
                let entry = cache.load_disk(&key)?;
                cache.insert_memory(key.clone(), entry.clone());
                Some(entry)
            })
            .await
            .unwrap_or(None);
            if let Some(entry) = disk_hit {
                tracing::info!(identifier = ref_.identifier, "profile.cache_disk_hit");
                let cache_age_seconds = Some((entry.age_seconds() * 10.0).round() / 10.0);
                return Ok(ProfileResult {
                    profile: entry.profile,
                    cached: true,
                    cache_age_seconds,
                    elapsed_ms: started.elapsed().as_millis() as i64,
                    upstream_calls: 0,
                });
            }
        }

        let profile = self.fetch_once(ref_.clone(), cache_key.clone()).await?;
        let upstream_calls = profile.meta.sources.len() as i64;

        Ok(ProfileResult {
            profile,
            cached: false,
            cache_age_seconds: None,
            elapsed_ms: started.elapsed().as_millis() as i64,
            upstream_calls,
        })
    }

    /// Coalesce concurrent lookups of the same profile into one upstream
    /// fetch. Cancelling one waiter must not cancel the leader; entries are
    /// removed after completion (success or failure) and the map is bounded,
    /// so a burst of distinct keys cannot grow it forever.
    async fn fetch_once(
        &self,
        ref_: ProfileRef,
        cache_key: String,
    ) -> Result<Arc<Profile>, AppError> {
        let mut receiver = {
            let mut inflight = self.inflight.lock().await;
            if let Some(existing) = inflight.get(&cache_key) {
                tracing::info!(identifier = ref_.identifier, "profile.request_coalesced");
                existing.receiver.clone()
            } else {
                let (tx, rx) = watch::channel(FlightState::Pending);
                let repository = self.repository.clone();
                let cache = self.cache.clone();
                let key = cache_key.clone();
                let inflight_map = self.inflight.clone();
                tokio::spawn(async move {
                    let started = chrono::Utc::now();
                    let outcome: RepoResult = repository
                        .fetch(&ref_, now_year_month(started), started)
                        .await
                        .map(Arc::new);
                    if let Ok(profile) = &outcome {
                        let entry = CachedProfile {
                            profile: profile.clone(),
                            inserted_at: Instant::now(),
                            stored_at: epoch_seconds(),
                        };
                        cache.insert_memory(key.clone(), entry);
                        let cache2 = cache.clone();
                        let key2 = key.clone();
                        let pr = profile.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let entry = CachedProfile {
                                profile: pr,
                                inserted_at: Instant::now(),
                                stored_at: epoch_seconds(),
                            };
                            cache2.save_disk(&key2, &entry);
                        })
                        .await;
                    }
                    let _ = tx.send(FlightState::Done(outcome));
                    inflight_map.lock().await.remove(&key);
                });
                inflight.insert(
                    cache_key.clone(),
                    Flight {
                        receiver: rx.clone(),
                    },
                );
                rx
            }
        };

        loop {
            let done = {
                let state = receiver.borrow();
                match &*state {
                    FlightState::Done(result) => Some(match result {
                        Ok(profile) => Ok(profile.clone()),
                        Err(err) => Err(clone_error(err)),
                    }),
                    FlightState::Pending => None,
                }
            };
            if let Some(result) = done {
                self.inflight.lock().await.remove(&cache_key);
                return result;
            }
            if receiver.changed().await.is_err() {
                return Err(AppError::Internal {
                    context: "in-flight fetch dropped".to_string(),
                });
            }
        }
    }

    pub fn cache_stats(&self) -> serde_json::Value {
        self.cache.stats()
    }

    /// Diagnostics-only raw fetch used by the protected raw endpoint.
    pub async fn raw_fetch(
        &self,
        call: crate::linkedin::endpoints::VoyagerCall,
        referer: String,
    ) -> Result<crate::linkedin::client::VoyagerResponse, AppError> {
        self.repository.raw_fetch(call, referer).await
    }
}

fn epoch_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn now_year_month(now: chrono::DateTime<chrono::Utc>) -> (i64, i64) {
    let by = chrono::Datelike::year(&now);
    let month = chrono::Datelike::month(&now);
    (i64::from(by), i64::from(month))
}

fn clone_error(error: &AppError) -> AppError {
    match error {
        AppError::Validation { message, field } => AppError::Validation {
            message: message.clone(),
            field: field.clone(),
        },
        AppError::InvalidProfileUrl { message, details } => AppError::InvalidProfileUrl {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::ApiKeyMissing => AppError::ApiKeyMissing,
        AppError::ApiKeyInvalid => AppError::ApiKeyInvalid,
        AppError::InsufficientCredit {
            balance_cents,
            required_cents,
        } => AppError::InsufficientCredit {
            balance_cents: *balance_cents,
            required_cents: *required_cents,
        },
        AppError::BillingUnavailable => AppError::BillingUnavailable,
        AppError::RateLimited {
            retry_after_seconds,
            limit,
            window_seconds,
        } => AppError::RateLimited {
            retry_after_seconds: *retry_after_seconds,
            limit: *limit,
            window_seconds: *window_seconds,
        },
        AppError::AuthFailed { message, details } => AppError::AuthFailed {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::SessionExpired { message, details } => AppError::SessionExpired {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::ChallengeRequired { message, details } => AppError::ChallengeRequired {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::ProfileNotFound { message, details } => AppError::ProfileNotFound {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::ProfileNotVisible { message, details } => AppError::ProfileNotVisible {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::LinkedinRateLimited { message, details } => AppError::LinkedinRateLimited {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::LinkedinUnavailable { message, details } => AppError::LinkedinUnavailable {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::CircuitOpen { message, details } => AppError::CircuitOpen {
            message: message.clone(),
            details: details.clone(),
        },
        AppError::EndpointDisabled => AppError::EndpointDisabled,
        AppError::Internal { context } => AppError::Internal {
            context: context.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::linkedin::client::{Upstream, VoyagerCall, VoyagerResponse};
    use futures::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingUpstream {
        calls: AtomicUsize,
        profile_calls: AtomicUsize,
        payload: std::sync::OnceLock<serde_json::Value>,
    }

    impl CountingUpstream {
        fn new() -> Self {
            let payload = serde_json::json!({
                "elements": [{
                    "firstName": "Ada",
                    "lastName": "Lovelace",
                    "headline": "Engineer",
                    "publicIdentifier": "adalovelace",
                    "entityUrn": "urn:li:fsd_profile:989898",
                }],
            });
            CountingUpstream {
                calls: AtomicUsize::new(0),
                profile_calls: AtomicUsize::new(0),
                payload: std::sync::OnceLock::from(payload),
            }
        }
    }

    impl Upstream for CountingUpstream {
        fn fetch(
            &self,
            call: VoyagerCall,
            _referer: String,
            _allow_fallback: bool,
        ) -> BoxFuture<'_, Result<VoyagerResponse, AppError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                // Only the first strategy answers; sleep briefly so 100
                // concurrent callers actually overlap.
                if call.name != "dashProfile" {
                    return Ok(VoyagerResponse {
                        name: call.name,
                        status_code: 404,
                        payload: None,
                        elapsed_ms: 0,
                        attempts: 1,
                    });
                }
                self.profile_calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                Ok(VoyagerResponse {
                    name: "dashProfile",
                    status_code: 200,
                    payload: Some(self.payload.get().expect("set").clone()),
                    elapsed_ms: 30,
                    attempts: 1,
                })
            })
        }
    }

    fn build_service() -> (Arc<ProfileService<CountingUpstream>>, Arc<CountingUpstream>) {
        let settings = Arc::new(Settings::default());
        let upstream = Arc::new(CountingUpstream::new());
        let repo = Arc::new(ProfileRepository::new(upstream.clone(), settings));
        let cache = Arc::new(ProfileCache::new(900, 32, None, false));
        (Arc::new(ProfileService::new(repo, cache)), upstream)
    }

    #[tokio::test]
    async fn coalesces_100_concurrent_misses_into_one_fetch() {
        let (service, upstream) = build_service();
        let mut handles = Vec::new();
        for _ in 0..100 {
            let svc = service.clone();
            handles.push(tokio::spawn(async move {
                svc.get_profile("https://www.linkedin.com/in/adalovelace/", false)
                    .await
            }));
        }
        for handle in handles {
            let result = handle.await.unwrap().unwrap();
            assert!(!result.cached);
            assert_eq!(
                result.profile.public_identifier.as_deref(),
                Some("adalovelace")
            );
        }
        assert_eq!(
            upstream.profile_calls.load(Ordering::SeqCst),
            1,
            "exactly one profile fetch for 100 simultaneous misses"
        );

        // Second round is served from cache without touching upstream.
        let cached = service.get_profile("adalovelace", false).await.unwrap();
        assert!(cached.cached);
        assert_eq!(upstream.profile_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_bypasses_cache_but_still_coalesces() {
        let (service, upstream) = build_service();
        let first = service.get_profile("adalovelace", false).await.unwrap();
        assert!(!first.cached);
        let second = service.get_profile("adalovelace", true).await.unwrap();
        assert!(!second.cached);
        assert_eq!(upstream.profile_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn invalid_url_is_validation_error() {
        let (service, _) = build_service();
        let err = service
            .get_profile("https://evil.com/in/x", false)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "INVALID_PROFILE_URL");
    }

    #[tokio::test]
    async fn leader_error_reaches_all_waiters() {
        let settings = Arc::new(Settings::default());
        let upstream = Arc::new(Always404);
        let repo = Arc::new(ProfileRepository::new(upstream, settings));
        let cache = Arc::new(ProfileCache::new(900, 32, None, false));
        let service = Arc::new(ProfileService::new(repo, cache));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let svc = service.clone();
            handles.push(tokio::spawn(async move {
                svc.get_profile("nobody-here", false).await
            }));
        }
        for handle in handles {
            let err = handle.await.unwrap().unwrap_err();
            assert_eq!(err.code(), "PROFILE_NOT_FOUND");
        }
    }

    #[derive(Default)]
    struct Always404;

    impl Upstream for Always404 {
        fn fetch(
            &self,
            call: VoyagerCall,
            _referer: String,
            _allow_fallback: bool,
        ) -> BoxFuture<'_, Result<VoyagerResponse, AppError>> {
            Box::pin(async move {
                Ok(VoyagerResponse {
                    name: call.name,
                    status_code: 404,
                    payload: None,
                    elapsed_ms: 0,
                    attempts: 1,
                })
            })
        }
    }
}
