//! Fetch orchestration for a single profile.
//!
//! Two stages: a **strategy chain** (decorated Dash, minimal Dash, GraphQL if
//! configured, then the retired legacy projection — stop at the first draft
//! with content) and **enrichment** (bounded-concurrent optional calls that
//! add skills/contact/network, allowed to fail without failing the request).

use std::sync::Arc;

use futures::future::join_all;
use serde_json::{Map, Value};
use tokio::sync::Semaphore;

use crate::config::Settings;

use super::client::{Upstream, VoyagerResponse};
use super::endpoints::{self, VoyagerCall};
use super::error as lerr;
use super::url::ProfileRef;
use crate::domain::profile::SourceCall;
use crate::error::AppError;
use crate::parser::assembler;

pub struct ProfileRepository<U: Upstream> {
    client: Arc<U>,
    enrichment_semaphore: Arc<Semaphore>,
}

impl<U: Upstream> ProfileRepository<U> {
    pub fn new(client: Arc<U>, settings: Arc<Settings>) -> Self {
        // Enrichment runs concurrently but stays bounded; the shared upstream
        // semaphore inside the client is the true concurrency bound.
        ProfileRepository {
            client,
            enrichment_semaphore: Arc::new(Semaphore::new(settings.upstream_max_concurrency)),
        }
    }

    pub async fn fetch(
        &self,
        ref_: &ProfileRef,
        now: (i64, i64),
        fetched_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::domain::profile::Profile, AppError> {
        let referer = ref_.canonical_url.clone();
        let mut payloads: Map<String, Value> = Map::new();
        let sources: std::sync::Mutex<Vec<SourceCall>> = std::sync::Mutex::new(Vec::new());
        let warnings: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

        let draft = self
            .fetch_profile(ref_, &referer, &mut payloads, &sources, &warnings, now)
            .await?;

        let draft = match draft {
            Some(draft) => draft,
            None => {
                let guard = sources.lock().unwrap();
                let attempted: Vec<&str> = guard.iter().map(|c| c.endpoint.as_str()).collect();
                return Err(lerr::profile_not_visible(&ref_.identifier, &attempted));
            }
        };

        self.enrich(&draft, &referer, &mut payloads, &sources, &warnings)
            .await;
        let mut draft = draft;
        assembler::enrich(&mut draft, &payloads);

        let populated = draft.populated_sections().len();
        tracing::info!(
            identifier = ref_.identifier,
            strategy = draft.strategy,
            sections = populated,
            "repository.profile_built"
        );

        Ok(assembler::build_profile(
            &ref_.identifier,
            ref_.kind.as_str(),
            &ref_.canonical_url,
            &draft,
            sources.into_inner().unwrap(),
            warnings.into_inner().unwrap(),
            fetched_at,
        ))
    }

    /// Diagnostics-only raw fetch: one upstream call without orchestration.
    pub async fn raw_fetch(
        &self,
        call: VoyagerCall,
        referer: String,
    ) -> Result<VoyagerResponse, AppError> {
        self.client.fetch(call, referer, true).await
    }

    // ------------------------------------------------------------- strategies

    fn strategies(&self, ref_: &ProfileRef) -> Vec<VoyagerCall> {
        // ponytail: ONE dash call per request (+ minimal fallback). Legacy
        // /identity/profiles/* calls (profileView, core, contact, skills,
        // network) all 410/302 now and the burst trips LinkedIn's protection,
        // soft-banning the whole session (verified live 2026-08-30: single
        // dash call 200, backend burst → every call 302 afterwards).
        vec![
            endpoints::dash_profile(&ref_.identifier),
            endpoints::dash_profile_minimal(&ref_.identifier),
        ]
    }

    async fn fetch_profile(
        &self,
        ref_: &ProfileRef,
        referer: &str,
        payloads: &mut Map<String, Value>,
        sources: &std::sync::Mutex<Vec<SourceCall>>,
        warnings: &std::sync::Mutex<Vec<String>>,
        now: (i64, i64),
    ) -> Result<Option<crate::parser::draft::ProfileDraft>, AppError> {
        let mut last_error: Option<AppError> = None;

        for call in self.strategies(ref_) {
            let response = match self
                .client
                .fetch(call.clone(), referer.to_string(), true)
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    // A definitive answer (404, throttling, dead session) is not
                    // something another strategy can improve on.
                    if err.status_code().is_client_error()
                        || err.status_code() == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || err.status_code() == reqwest::StatusCode::SERVICE_UNAVAILABLE
                    {
                        return Err(err);
                    }
                    last_error = Some(err);
                    sources.lock().unwrap().push(SourceCall {
                        endpoint: call.name.to_string(),
                        status_code: 0,
                        ok: false,
                        elapsed_ms: 0,
                        attempts: 1,
                    });
                    continue;
                }
            };

            record(&response, payloads, sources);

            if !response.ok() && response.status_code == 404 {
                return Err(lerr::profile_not_found(call.name));
            }
            if response.payload.is_none() {
                warnings.lock().unwrap().push(warning_for(
                    call.name,
                    Some(response.status_code),
                    None,
                ));
                continue;
            }

            let draft = if call.name == "profileView" {
                assembler::draft_from_legacy(response.payload.as_ref(), None, now)
            } else {
                assembler::draft_from_dash(response.payload.as_ref(), call.name, now)
            };
            if let Some(draft) = draft {
                return Ok(Some(draft));
            }
            warnings.lock().unwrap().push(format!(
                "Profile model '{}' answered but carried no profile entity.",
                call.name
            ));
        }

        if let Some(error) = last_error {
            return Err(error);
        }
        Ok(None)
    }

    // ------------------------------------------------------------- enrichment

    /// Run the optional enrichment calls concurrently but bounded, folding each
    /// success or failure into `payloads`/`warnings` without failing the
    /// profile request.
    async fn enrich(
        &self,
        draft: &crate::parser::draft::ProfileDraft,
        referer: &str,
        payloads: &mut Map<String, Value>,
        sources: &std::sync::Mutex<Vec<SourceCall>>,
        warnings: &std::sync::Mutex<Vec<String>>,
    ) {
        // ponytail: dash contact only. Legacy enrichment endpoints trigger
        // session bans (see strategies()); add back individually with pacing
        // if a ban-free pattern is ever proven.
        let mut optional = Vec::new();
        if let Some(profile_id) = draft.identity.profile_id.as_ref() {
            optional.push(endpoints::dash_contact_info(profile_id));
        }

        let results = join_all(
            optional
                .iter()
                .map(|call| self.enrich_one(call.clone(), referer.to_string())),
        )
        .await;

        for (call, result) in optional.iter().zip(results) {
            match result {
                Ok(response) => {
                    record(&response, payloads, sources);
                    if !response.ok() {
                        warnings.lock().unwrap().push(warning_for(
                            call.name,
                            Some(response.status_code),
                            None,
                        ));
                    }
                }
                Err(err) => {
                    warnings
                        .lock()
                        .unwrap()
                        .push(warning_for(call.name, None, Some(err.code())));
                    sources.lock().unwrap().push(SourceCall {
                        endpoint: call.name.to_string(),
                        status_code: 0,
                        ok: false,
                        elapsed_ms: 0,
                        attempts: 1,
                    });
                }
            }
        }
    }

    /// Each enrichment call is optional: failures come back as `Err`, handled
    /// by the caller as warnings, never as request failures.
    async fn enrich_one(
        &self,
        call: VoyagerCall,
        referer: String,
    ) -> Result<VoyagerResponse, AppError> {
        let _permit = self
            .enrichment_semaphore
            .acquire()
            .await
            .expect("semaphore not closed");
        self.client.fetch(call.clone(), referer, false).await
    }
}

fn record(
    response: &VoyagerResponse,
    payloads: &mut Map<String, Value>,
    sources: &std::sync::Mutex<Vec<SourceCall>>,
) {
    let _ = payloads.insert(
        response.name.to_string(),
        response.payload.clone().unwrap_or(Value::Null),
    );
    sources.lock().unwrap().push(SourceCall {
        endpoint: response.name.to_string(),
        status_code: response.status_code,
        ok: response.ok(),
        elapsed_ms: response.elapsed_ms,
        attempts: response.attempts,
    });
}

const HINTS: &[(&str, &str)] = &[
    (
        "contactInfo",
        "contact details are not shared with the querying account",
    ),
    (
        "dashContactInfo",
        "contact details are not shared with the querying account",
    ),
    (
        "skills",
        "endorsement counts are unavailable; profile skills were used",
    ),
    (
        "networkInfo",
        "follower and connection counts are unavailable",
    ),
    (
        "profileView",
        "the legacy profile projection has been retired by LinkedIn",
    ),
    (
        "dashProfile",
        "the decorated profile collection did not answer",
    ),
    (
        "dashProfileMinimal",
        "the undecorated profile collection did not answer",
    ),
    (
        "graphqlProfile",
        "the configured GraphQL queryId did not answer",
    ),
];

fn warning_for(endpoint: &str, status: Option<i64>, code: Option<&str>) -> String {
    let reason = code.map(str::to_string).unwrap_or_else(|| {
        status.map_or_else(|| "no response".to_string(), |s| format!("HTTP {s}"))
    });
    let message = format!("Call '{endpoint}' unavailable ({reason})");
    let hint = HINTS
        .iter()
        .find(|(name, _)| *name == endpoint)
        .map(|(_, hint)| *hint);
    match hint {
        Some(hint) => format!("{message}: {hint}"),
        None => message,
    }
}
