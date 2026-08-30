//! Service/repository flow tests: strategy order, fallback warnings,
//! definitive errors, enrichment failure isolation, provenance, and refresh.

mod support;

use std::sync::Arc;

use serde_json::json;

use support::{MockUpstream, fixture};
use tross::config::Settings;
use tross::linkedin::repository::ProfileRepository;
use tross::service::cache::ProfileCache;
use tross::service::profile::ProfileService;

fn service_with(mock: &Arc<MockUpstream>) -> Arc<ProfileService<MockUpstream>> {
    let settings = Arc::new(Settings::default());
    let repo = Arc::new(ProfileRepository::new(mock.clone(), settings.clone()));
    let cache = Arc::new(ProfileCache::new(900, 32, None, false));
    Arc::new(ProfileService::new(repo, cache))
}

const URL: &str = "https://www.linkedin.com/in/adalovelace/";

#[tokio::test]
async fn strategy_stops_at_first_populated_draft() {
    let mock = MockUpstream::new();
    let dash = fixture("dash_normalized.json");
    // Minimal Dash answers 200 with the same profile, but it must never be
    // called: the decorated Dash already produced a draft.
    mock.respond("dashProfile", 200, Some(dash));
    mock.respond("dashContactInfo", 200, Some(fixture("contact_info.json")));

    let service = service_with(&mock);
    let result = service.get_profile(URL, false).await.unwrap();
    let profile = &result.profile;
    assert_eq!(profile.first_name.as_deref(), Some("Ada"));
    assert_eq!(profile.meta.sources[0].endpoint, "dashProfile");
    assert_eq!(
        mock.strategy_calls(),
        1,
        "strategy chain stopped immediately"
    );

    let proxy = serde_json::to_value(
        profile
            .meta
            .sources
            .iter()
            .map(|s| s.endpoint.as_str())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let rendered = proxy.to_string();
    assert!(rendered.contains("dashProfile"));
    assert!(rendered.contains("dashContactInfo"));
    assert!(!rendered.contains("contactInfo"));
    assert!(!rendered.contains("skills"));
    assert!(!rendered.contains("networkInfo"));
}

#[tokio::test]
async fn empty_dash_falls_back_with_warning() {
    let mock = MockUpstream::new();
    // Decorated Dash answers 200 but carries NO profile entity.
    mock.respond(
        "dashProfile",
        200,
        Some(json!({"data": {}, "included": []})),
    );
    mock.respond(
        "dashProfileMinimal",
        200,
        Some(fixture("dash_normalized.json")),
    );
    mock.respond("dashContactInfo", 200, Some(fixture("contact_info.json")));

    let service = service_with(&mock);
    let result = service.get_profile(URL, false).await.unwrap();
    let profile = &result.profile;
    assert_eq!(profile.first_name.as_deref(), Some("Ada"), "fallback used");
    let warnings = profile
        .meta
        .warnings
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("answered but carried no profile entity")),
        "warning recorded: {warnings:?}"
    );
    assert_eq!(mock.strategy_calls(), 2, "decorated then minimal");
}

#[tokio::test]
async fn definitive_404_stops_the_chain() {
    let mock = MockUpstream::new();
    mock.respond("dashProfile", 404, None);

    let service = service_with(&mock);
    let err = service.get_profile(URL, false).await.unwrap_err();
    assert_eq!(err.code(), "PROFILE_NOT_FOUND");
    assert_eq!(
        mock.strategy_calls(),
        1,
        "definitive 404 stops the chain immediately"
    );
}

#[tokio::test]
async fn enrichment_failure_becomes_warning_not_error() {
    let mock = MockUpstream::new();
    mock.respond("dashProfile", 200, Some(fixture("dash_normalized.json")));
    mock.respond("dashContactInfo", 404, None);

    let service = service_with(&mock);
    let result = service.get_profile(URL, false).await.unwrap();
    let profile = &result.profile;
    assert_eq!(
        profile.first_name.as_deref(),
        Some("Ada"),
        "main response intact"
    );
    let warnings = profile
        .meta
        .warnings
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert!(
        warnings.iter().any(|w| w.contains("dashContactInfo")),
        "contact warning: {warnings:?}"
    );
}

#[tokio::test]
async fn provenance_and_completeness_recorded() {
    let mock = MockUpstream::new();
    mock.respond("dashProfile", 200, Some(fixture("dash_normalized.json")));
    mock.respond("dashContactInfo", 200, Some(fixture("contact_info.json")));

    let service = service_with(&mock);
    let result = service.get_profile(URL, false).await.unwrap();
    let profile = &result.profile;
    assert_eq!(profile.meta.sources.len(), 2, "dash + contact enrichment");
    assert_eq!(profile.meta.completeness, 1.0);
    assert_eq!(profile.meta.sections_populated.len(), 13);
}

#[tokio::test]
async fn refresh_invalidates_cache() {
    let mock = MockUpstream::new();
    mock.respond("dashProfile", 200, Some(fixture("dash_normalized.json")));
    mock.respond("dashContactInfo", 200, Some(fixture("contact_info.json")));

    let service = service_with(&mock);
    let first = service.get_profile(URL, false).await.unwrap();
    assert!(!first.cached);
    let cached = service.get_profile(URL, false).await.unwrap();
    assert!(cached.cached);
    let refreshed = service.get_profile(URL, true).await.unwrap();
    assert!(!refreshed.cached, "refresh bypasses the cache");
    let cached_again = service.get_profile(URL, false).await.unwrap();
    assert!(cached_again.cached);
}
