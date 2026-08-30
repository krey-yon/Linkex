//! Politeness and failure-isolation primitives for upstream calls:
//! a spacing throttle with jitter and concurrency ceiling, and a three-state
//! circuit breaker with exactly one half-open probe.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use rand::Rng;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio::time::Instant;

use crate::error::AppError;

/// Serialises upstream calls to a minimum spacing with random jitter.
///
/// The spacing reservation is computed while holding a short mutex (with
/// monotonic `Instant`), then the wait sleeps outside the lock. Owned
/// semaphore permits make cancellation release capacity automatically.
pub struct RequestThrottle {
    semaphore: Arc<Semaphore>,
    min_interval: Duration,
    jitter: Duration,
    next_allowed_at: Mutex<Instant>,
}

impl RequestThrottle {
    pub fn new(min_interval_seconds: f64, jitter_seconds: f64, max_concurrency: usize) -> Self {
        let min_interval = Duration::from_secs_f64(min_interval_seconds.max(0.0));
        let jitter = Duration::from_secs_f64(jitter_seconds.max(0.0));
        RequestThrottle {
            semaphore: Arc::new(Semaphore::new(max_concurrency.max(1))),
            min_interval,
            jitter,
            next_allowed_at: Mutex::new(Instant::now()),
        }
    }

    /// Acquire one upstream slot, sleeping only the *prior* wait (matching the
    /// Python behaviour: the reservation advances the schedule, the sleep is
    /// what was owed at submission time).
    pub async fn acquire(&self) -> OwnedPermitGuard {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore not closed");
        let now = Instant::now();
        let jitter = Duration::from_secs_f64(if self.jitter.is_zero() {
            0.0
        } else {
            rand::rng().random_range(0.0..self.jitter.as_secs_f64())
        });
        let wait_for = {
            let mut next_allowed_at = self.next_allowed_at.lock().unwrap();
            let wait_for = next_allowed_at.saturating_duration_since(now);
            let spacing = self.min_interval + jitter;
            *next_allowed_at = (*next_allowed_at).max(now) + spacing;
            wait_for
        };

        if !wait_for.is_zero() {
            tokio::time::sleep(wait_for).await;
        }
        OwnedPermitGuard(Some(permit))
    }
}

pub struct OwnedPermitGuard(Option<tokio::sync::OwnedSemaphorePermit>);

impl Drop for OwnedPermitGuard {
    fn drop(&mut self) {
        self.0.take();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl BreakerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            BreakerState::Closed => "closed",
            BreakerState::Open => "open",
            BreakerState::HalfOpen => "half-open",
        }
    }
}

enum Inner {
    Closed {
        failures: usize,
    },
    Open {
        opened_at: Instant,
    },
    /// Exactly one caller is probing; all others fail fast.
    HalfOpen,
}

/// Classic three-state breaker with a single probe:
/// - failures at or above `threshold` open it;
/// - open callers fail fast until the cooldown passes;
/// - then exactly one caller gets a half-open probe; success closes, failure
///   reopens with a fresh cooldown.
pub struct CircuitBreaker {
    threshold: usize,
    cooldown: Duration,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(threshold: usize, cooldown_seconds: f64) -> Self {
        CircuitBreaker {
            threshold: threshold.max(1),
            cooldown: Duration::from_secs_f64(cooldown_seconds.max(0.0)),
            inner: Mutex::new(Inner::Closed { failures: 0 }),
        }
    }

    /// Fail fast when open; grant exactly one half-open probe after cooldown.
    pub fn check(&self) -> Result<(), AppError> {
        let mut inner = self.inner.lock().unwrap();
        match *inner {
            Inner::Closed { .. } => Ok(()),
            Inner::Open { opened_at } => {
                if opened_at.elapsed() < self.cooldown {
                    let retry_in = (self.cooldown - opened_at.elapsed())
                        .as_secs_f64()
                        .round()
                        .max(0.0);
                    Err(AppError::CircuitOpen {
                        message: "Upstream calls are paused after repeated LinkedIn failures."
                            .to_string(),
                        details: retry_after_map(retry_in),
                    })
                } else {
                    *inner = Inner::HalfOpen;
                    Ok(())
                }
            }
            Inner::HalfOpen => Err(AppError::CircuitOpen {
                message: "Upstream calls are paused after repeated LinkedIn failures.".to_string(),
                details: retry_after_map(self.cooldown.as_secs_f64()),
            }),
        }
    }

    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        let changed = matches!(&*inner, Inner::Open { .. } | Inner::HalfOpen);
        *inner = Inner::Closed { failures: 0 };
        if changed {
            tracing::info!("circuit.closed");
        }
    }

    /// The failure threshold; tests drive the breaker with record_failure.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// The cooldown; exposed for diagnostics parity with the Python breaker.
    pub fn cooldown_seconds(&self) -> f64 {
        self.cooldown.as_secs_f64()
    }

    pub fn record_failure(&self) {
        let mut inner = self.inner.lock().unwrap();
        let event = match &*inner {
            Inner::Closed { failures } => {
                let failures = *failures + 1;
                if failures >= self.threshold {
                    *inner = Inner::Open {
                        opened_at: Instant::now(),
                    };
                    tracing::warn!(
                        failures = failures,
                        cooldown = self.cooldown.as_secs_f64(),
                        "circuit.opened"
                    );
                } else {
                    *inner = Inner::Closed { failures };
                }
                false
            }
            Inner::HalfOpen => {
                // The probe failed: reopen with a fresh cooldown.
                *inner = Inner::Open {
                    opened_at: Instant::now(),
                };
                true
            }
            Inner::Open { .. } => false,
        };
        if event {
            tracing::warn!("circuit.reopened_after_probe");
        }
    }

    pub fn state(&self) -> BreakerState {
        let inner = self.inner.lock().unwrap();
        match *inner {
            Inner::Closed { .. } => BreakerState::Closed,
            Inner::Open { opened_at } => {
                if opened_at.elapsed() >= self.cooldown {
                    BreakerState::HalfOpen
                } else {
                    BreakerState::Open
                }
            }
            Inner::HalfOpen => BreakerState::HalfOpen,
        }
    }

    /// Redacted diagnostics, never operationally sensitive.
    pub fn snapshot(&self) -> serde_json::Value {
        let inner = self.inner.lock().unwrap();
        let (state, failures) = match *inner {
            Inner::Closed { failures } => (BreakerState::Closed, failures),
            Inner::Open { opened_at } => {
                if opened_at.elapsed() >= self.cooldown {
                    (BreakerState::HalfOpen, 1)
                } else {
                    (BreakerState::Open, 1)
                }
            }
            Inner::HalfOpen => (BreakerState::HalfOpen, 0),
        };
        json!({
            "state": state.as_str(),
            "consecutive_failures": failures,
        })
    }
}

fn retry_after_map(seconds: f64) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([("retry_after_seconds".to_string(), json!(seconds))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Arc;

    macro_rules! time_pass {
        ($ms:expr) => {
            tokio::time::advance(Duration::from_millis($ms)).await;
            tokio::task::yield_now().await;
        };
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_enforces_spacing_and_jitter_bounds() {
        let throttle = Arc::new(RequestThrottle::new(1.0, 0.5, 2));
        let t = throttle.clone();
        let first = tokio::spawn(async move {
            let started = tokio::time::Instant::now();
            let _guard = t.acquire().await;
            started.elapsed().as_millis() as i64
        });
        time_pass!(50);
        let second = tokio::spawn(async move {
            let started = tokio::time::Instant::now();
            let _guard = throttle.acquire().await;
            started.elapsed().as_millis() as i64
        });
        time_pass!(200);

        let waited_first = first.await.unwrap();
        assert!(
            waited_first <= 100,
            "leader should sleep only what it owed: {waited_first}"
        );
        // The second acquire sees the frozen schedule: between min (1000ms)
        // and min+jitter (1500ms) after time 0, so ~750-1500ms of sleep.
        let waited_second = second.await.unwrap();
        assert!(
            (700..=1600).contains(&waited_second),
            "waiter waited {waited_second}ms"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_limits_concurrency() {
        let throttle = Arc::new(RequestThrottle::new(0.0, 0.0, 1));
        let t1 = throttle.clone();
        let first = tokio::spawn(async move {
            let guard = t1.acquire().await;
            tokio::time::sleep(Duration::from_millis(60_000)).await;
            drop(guard);
        });
        time_pass!(10);

        let t2 = throttle.clone();
        let second = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(1000), t2.acquire()).await
        });
        time_pass!(200);
        assert!(
            !second.is_finished(),
            "second request must wait for the permit"
        );

        first.abort();
        time_pass!(200);
        let outcome = second.await.unwrap();
        assert!(
            outcome.is_ok(),
            "second request should acquire once the holder exits"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn breakers_open_and_probe_once() {
        let breaker = CircuitBreaker::new(2, 90.0);
        assert_eq!(breaker.state(), BreakerState::Closed);
        assert!(breaker.check().is_ok());
        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.state(), BreakerState::Open);
        assert!(breaker.check().is_err());
        let snap = breaker.snapshot();
        assert_eq!(snap["state"], Value::String("open".into()));

        time_pass!(95_000);
        assert_eq!(breaker.state(), BreakerState::HalfOpen);
        assert!(breaker.check().is_ok(), "first caller gets the probe");
        assert!(breaker.check().is_err(), "others still fail fast");
        breaker.record_failure();
        assert_eq!(breaker.state(), BreakerState::Open);

        time_pass!(95_000);
        assert!(breaker.check().is_ok());
        breaker.record_success();
        assert_eq!(breaker.state(), BreakerState::Closed);
        breaker.record_failure();
        assert_eq!(
            breaker.state(),
            BreakerState::Closed,
            "one failure below threshold"
        );
        breaker.record_failure();
        assert_eq!(breaker.state(), BreakerState::Open, "threshold reached");
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_releases_permit() {
        let throttle = Arc::new(RequestThrottle::new(0.0, 0.0, 1));
        let t1 = throttle.clone();
        let handle = tokio::spawn(async move {
            let guard = t1.acquire().await;
            tokio::time::sleep(Duration::from_millis(60_000)).await;
            drop(guard);
        });
        time_pass!(10);
        handle.abort();
        time_pass!(10);

        let t2 = throttle.clone();
        let second = tokio::spawn(async move { t2.acquire().await });
        time_pass!(10);
        assert!(second.await.is_ok(), "permit released by cancellation");
    }
}
