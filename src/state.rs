//! Shared application state: cheap clones / `Arc` handles only.
//! No giant mutex around application state.

use std::sync::Arc;
use std::time::Instant;

use crate::billing::BillingStore;
use crate::config::Settings;
use crate::linkedin::client::{SessionDiagnostics, Upstream};
use crate::service::profile::ProfileService;

pub struct AppState<U: Upstream> {
    pub settings: Arc<Settings>,
    pub service: Arc<ProfileService<U>>,
    pub voyager: Arc<dyn SessionDiagnostics>,
    pub billing: BillingStore,
    pub rate_limiter: Arc<crate::api::middleware::RateLimiter>,
    pub started_at: Instant,
}

impl<U: Upstream> Clone for AppState<U> {
    fn clone(&self) -> Self {
        AppState {
            settings: self.settings.clone(),
            service: self.service.clone(),
            voyager: self.voyager.clone(),
            billing: self.billing.clone(),
            rate_limiter: self.rate_limiter.clone(),
            started_at: self.started_at,
        }
    }
}
