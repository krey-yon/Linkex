//! In-process API contract tests: routes, envelopes, headers, auth, rate
//! limits, and errors, all against mocked upstream responses.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

use support::{MockUpstream, build_app_with, fixture};
use tross::config::Settings;

fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn request_with<P: serde::Serialize + ?Sized>(method: &str, uri: &str, body: &P) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

struct Response {
    status: StatusCode,
    body: Value,
    headers: axum::http::HeaderMap,
}

async fn run(router: axum::Router, req: Request<Body>) -> Response {
    use axum::extract::connect_info::ConnectInfo;
    let app = router.into_service();
    let mut req = req;
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:5555".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Response {
        status,
        body,
        headers,
    }
}

fn router_with(mock: &Arc<MockUpstream>) -> axum::Router {
    let payload = fixture("dash_normalized.json");
    mock.respond("dashProfile", 200, Some(payload));
    mock.respond("contactInfo", 200, Some(fixture("contact_info.json")));
    mock.respond(
        "skills",
        200,
        Some(json!({"elements": [{"name": "Rust", "endorsementCount": 42}]})),
    );
    mock.respond(
        "networkInfo",
        200,
        Some(json!({"followersCount": 4810, "connectionsCount": 187})),
    );
    mock.respond("dashContactInfo", 200, Some(fixture("contact_info.json")));
    build_app_with(Settings::default(), mock.clone())
}

const PROFILE_URL: &str = "https://www.linkedin.com/in/adalovelace/";

#[tokio::test]
async fn get_profile_contract() {
    let mock = MockUpstream::new();
    let router = router_with(&mock);
    let mut req = request("GET", &format!("/v1/profile?url={PROFILE_URL}"));
    req.headers_mut()
        .insert("x-request-id", "test-123".parse().unwrap());
    let res = run(router, req).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.headers.get("x-cache").unwrap(), "MISS");
    assert_eq!(
        res.headers.get("x-request-id").unwrap(),
        "test-123",
        "inbound request id is echoed"
    );
    assert!(res.headers.contains_key("x-response-time-ms"));

    assert_eq!(res.body["success"], true, "success envelope");
    let meta = &res.body["meta"];
    assert_eq!(meta["cached"], false);
    assert_eq!(meta["request_id"], "test-123");
    let calls = meta["upstream_calls"].as_i64().unwrap();
    assert_eq!(
        calls, 2,
        "one dash strategy call + dashContactInfo enrichment: {calls}"
    );

    let data = &res.body["data"];
    assert_eq!(
        data["profile_url"],
        "https://www.linkedin.com/in/adalovelace/"
    );
    assert_eq!(data["first_name"], "Ada");
    assert_eq!(data["experience"][0]["title"], "Analytical Engineer");
    assert_eq!(data["meta"]["completeness"], 1.0);
    assert_eq!(
        data["meta"]["sections_populated"].as_array().unwrap().len(),
        13
    );
    assert_eq!(data["contact"]["emails"][0], "ada.lovelace@example.com");
    assert_eq!(
        data["skills"][0]["endorsement_count"], 42,
        "enrichment skills adopted"
    );
}

#[tokio::test]
async fn second_request_is_cached() {
    let mock = MockUpstream::new();
    let router = router_with(&mock);
    let first = run(
        router.clone(),
        request("GET", &format!("/v1/profile?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(first.headers.get("x-cache").unwrap(), "MISS");
    let calls_after_first = mock.total_calls();

    let second = run(
        router.clone(),
        request("GET", &format!("/v1/profile?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(second.headers.get("x-cache").unwrap(), "HIT");
    assert_eq!(second.body["meta"]["cached"], true);
    assert_eq!(
        mock.total_calls(),
        calls_after_first,
        "no upstream calls on cache hit"
    );
}

#[tokio::test]
async fn post_profile_refresh_works() {
    let mock = MockUpstream::new();
    let router = router_with(&mock);
    let res = run(
        router,
        request_with(
            "POST",
            "/v1/profile",
            &json!({"url": PROFILE_URL, "refresh": true}),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["meta"]["cached"], false);
    assert_eq!(res.body["data"]["first_name"], "Ada");
}

#[tokio::test]
async fn auth_api_key_required_and_bearer_ok() {
    let mut settings = Settings::default();
    settings.api_keys = vec!["k1".to_string()];
    let mock = MockUpstream::new();
    let payload = fixture("dash_normalized.json");
    mock.respond("dashProfile", 200, Some(payload));
    let router = build_app_with(settings, mock);

    // missing
    let res = run(
        router.clone(),
        request("GET", &format!("/v1/profile?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"]["code"], "API_KEY_MISSING");
    assert_eq!(res.body["success"], false);

    // wrong
    let mut req = request("GET", &format!("/v1/profile?url={PROFILE_URL}"));
    req.headers_mut()
        .insert("x-api-key", "nope".parse().unwrap());
    let res = run(router.clone(), req).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["error"]["code"], "API_KEY_INVALID");

    // header
    let mut req = request("GET", &format!("/v1/profile?url={PROFILE_URL}"));
    req.headers_mut().insert("x-api-key", "k1".parse().unwrap());
    let res = run(router.clone(), req).await;
    assert_eq!(res.status, StatusCode::OK);

    // bearer
    let mut req = request("GET", &format!("/v1/profile?url={PROFILE_URL}"));
    req.headers_mut()
        .insert("authorization", "Bearer k1".parse().unwrap());
    let res = run(router.clone(), req).await;
    assert_eq!(res.status, StatusCode::OK);

    // open when no keys configured
    let open_mock = MockUpstream::new();
    let payload = fixture("dash_normalized.json");
    open_mock.respond("dashProfile", 200, Some(payload));
    let open_router = build_app_with(Settings::default(), open_mock);
    let res = run(
        open_router,
        request("GET", &format!("/v1/profile?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn rate_limit_returns_retry_after() {
    let mut settings = Settings::default();
    settings.rate_limit_requests = 2;
    settings.rate_limit_window_seconds = 60;
    let mock = MockUpstream::new();
    let payload = fixture("dash_normalized.json");
    mock.respond("dashProfile", 200, Some(payload));
    let router = build_app_with(settings, mock);

    for _ in 0..2 {
        let res = run(
            router.clone(),
            request("GET", &format!("/v1/profile?url={PROFILE_URL}")),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "first two allowed");
    }
    let res = run(
        router.clone(),
        request("GET", &format!("/v1/profile?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"]["code"], "RATE_LIMITED");
    assert!(res.headers.contains_key("retry-after"));
}

#[tokio::test]
async fn error_envelopes() {
    let mock = MockUpstream::new();
    let router = router_with(&mock);

    // invalid profile url
    let res = run(
        router.clone(),
        request("GET", "/v1/profile?url=https://evil.com/in/x"),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["error"]["code"], "INVALID_PROFILE_URL");
    assert_eq!(res.body["success"], false);

    // unknown route
    let res = run(router.clone(), request("GET", "/nope")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"]["code"], "ROUTE_NOT_FOUND");

    // wrong method
    let res = run(router.clone(), request("POST", "/v1/session")).await;
    assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(res.body["error"]["code"], "METHOD_NOT_ALLOWED");
}

#[tokio::test]
async fn system_routes() {
    let mock = MockUpstream::new();
    let router = router_with(&mock);

    let res = run(router.clone(), request("GET", "/healthz")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["status"], "ok");
    assert!(res.body["version"].is_string());
    assert!(res.body["environment"].is_string());
    assert!(res.body["uptime_seconds"].as_f64().is_some());

    let res = run(router.clone(), request("GET", "/")).await;
    assert_eq!(res.status, StatusCode::OK);
    // `/` now serves the static SSR site instead of JSON metadata.
    assert!(
        res.headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .starts_with("text/html")
    );
    let asset = run(router.clone(), request("GET", "/style.css")).await;
    assert_eq!(asset.status, StatusCode::OK);

    let res = run(router.clone(), request("GET", "/readyz")).await;
    // Without LinkedIn credentials the session cannot be validated; readiness
    // must report 503 with a stable code rather than pretending to be ready.
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(res.body["ready"], false);
    assert_eq!(res.body["error"]["code"], "LINKEDIN_AUTH_FAILED");
}

#[tokio::test]
async fn session_endpoint_is_redacted() {
    let mut settings = Settings::default();
    settings.api_keys = vec!["k1".to_string()];
    let mock = MockUpstream::new();
    let router = build_app_with(settings, mock);

    let mut req = request("GET", "/v1/session");
    req.headers_mut().insert("x-api-key", "k1".parse().unwrap());
    let res = run(router, req).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.body["session"].is_object());
    assert!(res.body["circuit"].is_object());
    assert!(res.body["cache"].is_object());
    assert!(res.body["checked_at"].is_string());
    let rendered = res.body.to_string();
    assert!(
        !rendered.contains("li_at") || !rendered.contains("="),
        "no cookie values leaked"
    );
}

#[tokio::test]
async fn raw_endpoint_disabled_by_default_and_enabled_when_flagged() {
    // disabled
    let mock = MockUpstream::new();
    let router = build_app_with(Settings::default(), mock);
    let res = run(
        router.clone(),
        request("GET", &format!("/v1/profile/raw?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"]["code"], "ENDPOINT_DISABLED");

    // enabled
    let mut settings = Settings::default();
    settings.expose_raw_endpoint = true;
    let mock = MockUpstream::new();
    mock.respond(
        "profileView",
        200,
        Some(json!({"profile": {"firstName": "Ada"}})),
    );
    mock.respond("profile", 200, Some(json!({"firstName": "Ada"})));
    mock.respond("contactInfo", 200, Some(fixture("contact_info.json")));
    mock.respond("skills", 200, Some(json!({"elements": []})));
    mock.respond("networkInfo", 200, Some(json!({})));
    let router = build_app_with(settings, mock);
    let res = run(
        router,
        request("GET", &format!("/v1/profile/raw?url={PROFILE_URL}")),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["identifier"], "adalovelace");
    assert!(res.body["calls"]["profileView"].is_object());
}

#[tokio::test]
async fn malformed_post_json_is_422() {
    let mock = MockUpstream::new();
    let router = router_with(&mock);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/profile")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let res = run(router, req).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}
