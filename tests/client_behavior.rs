//! Client behavior without network: missing credentials and open circuit.

mod support;

use std::sync::Arc;

use tross::config::Settings;
use tross::linkedin::client::VoyagerClient;
use tross::linkedin::session::InMemoryStore;

#[tokio::test]
async fn missing_credentials_is_auth_failed() {
    let client = VoyagerClient::new(
        Arc::new(Settings::default()),
        Arc::new(InMemoryStore::shared()),
    );
    let err = client.ensure_session(false).await.unwrap_err();
    assert_eq!(err.code(), "LINKEDIN_AUTH_FAILED");
}

#[tokio::test]
async fn open_circuit_fails_before_any_network() {
    let client = VoyagerClient::new(
        Arc::new(Settings::default()),
        Arc::new(InMemoryStore::shared()),
    );
    // Drive the breaker open directly; then any fetch must fail fast.
    for _ in 0..client.breaker().threshold() {
        client.breaker().record_failure();
    }
    assert_eq!(client.breaker().state().as_str(), "open");

    let call = tross::linkedin::endpoints::dash_profile("adalovelace");
    let err = client
        .fetch(
            call,
            "https://www.linkedin.com/in/adalovelace/".to_string(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "UPSTREAM_CIRCUIT_OPEN");
}
