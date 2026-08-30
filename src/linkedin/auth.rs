//! Browserless authentication against LinkedIn.
//!
//! Two paths, in order of preference:
//!
//! 1. **Cookie injection** — the operator supplies a `li_at` cookie (ideally
//!    the whole `Cookie:` header) captured once from a signed-in browser. This
//!    is the production path: it never touches the login endpoint, so it never
//!    triggers e-mail verification or CAPTCHA challenges.
//! 2. **Credential login** — replays the form post against `/uas/authenticate`
//!    with a seeded `JSESSIONID` echoed back as the CSRF token. Off by default.
//!
//! The cookie jar is the only place cookies live; a per-request `Cookie`
//! header is never sent alongside the jar (duplicate `JSESSIONID` values break
//! CSRF).

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;

use crate::config::Settings;

use super::endpoints;
use super::error as lerr;
use super::session::{
    JSESSIONID, LI_AT, LinkedInSession, SessionSource, SessionStore, quote_cookie,
};

pub const COOKIE_DOMAIN: &str = "www.linkedin.com";

const LI_USER_AGENT: &str =
    "LIAuthLibrary:0.0.3 com.linkedin.android:4.1.881 Asus_ASUS_Z01QD:android_9";

pub struct Authenticator {
    settings: Arc<Settings>,
}

impl Authenticator {
    pub fn new(settings: Arc<Settings>) -> Self {
        Authenticator { settings }
    }

    // ------------------------------------------------------------ cookie jar

    /// Install the session in the client jar, replacing what was there.
    pub fn apply_session(
        &self,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        session: &LinkedInSession,
    ) {
        let mut store = jar.lock().unwrap();
        store.clear();
        for (name, value) in &session.cookies {
            if !value.is_empty() {
                let _ = store.parse(
                    &format!("{name}={value}; Domain={COOKIE_DOMAIN}; Path=/"),
                    &jar_url(),
                );
            }
        }
    }

    /// Copy cookies LinkedIn rotated back into the session.
    pub fn absorb_cookies(
        &self,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        session: &mut LinkedInSession,
    ) {
        let store = jar.lock().unwrap();
        let rotated: HashMap<String, String> = store
            .get_request_values(&jar_url())
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        session.merge_cookies(&rotated);
    }

    // ------------------------------------------------------------------ public

    /// Return a validated session, reusing a persisted one when possible. The
    /// client guards this with an async mutex so concurrent hoists cause one
    /// refresh, not a login stampede.
    pub async fn authenticate(
        &self,
        client: &Client,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        store: &Arc<dyn SessionStore>,
    ) -> Result<LinkedInSession, crate::error::AppError> {
        if let Some(mut restored) = store.load()
            && restored.is_usable()
            // ponytail: unwrap_or so a signed-out cache falls through to the env
            // cookie instead of aborting auth before fresh credentials are tried.
            && self.validate(client, jar, &mut restored).await.unwrap_or(false)
        {
            store.save(&restored);
            return Ok(restored);
        }

        let mut session = self.session_from_environment();
        if let Some(ref mut session) = session {
            if session.csrf_token().is_none() {
                tracing::warn!("auth.seeding_csrf_token_without_pair");
                self.ensure_csrf_token(client, jar, session).await?;
            }
            if self.validate(client, jar, session).await? {
                store.save(session);
                return Ok(session.clone());
            }
            // A stale JSESSIONID yields "CSRF check failed" rather than 401.
            if self.reseed_csrf_token(client, jar, session).await
                && self.validate(client, jar, session).await?
            {
                tracing::info!("auth.recovered_with_fresh_csrf_token");
                store.save(session);
                return Ok(session.clone());
            }
            tracing::warn!("auth.environment_cookie_rejected");
        }

        if self.settings.has_password_credentials() {
            let credentials = self.login_with_credentials(client, jar).await?;
            let mut credentials = credentials;
            if self.validate(client, jar, &mut credentials).await? {
                store.save(&credentials);
                return Ok(credentials);
            }
            return Err(lerr::auth_failed(
                "Login succeeded but the session failed validation.",
                "authenticate",
            ));
        }

        if session.is_some() {
            return Err(lerr::session_expired(
                "The LinkedIn session cookie is no longer valid; re-seed credentials.",
                "authenticate",
                None,
            ));
        }
        Err(lerr::auth_failed(
            "No LinkedIn credentials configured. Set LINKEDIN_LI_AT (recommended) or \
             LINKEDIN_EMAIL and LINKEDIN_PASSWORD.",
            "authenticate",
        ))
    }

    pub fn invalidate(&self, store: &Arc<dyn SessionStore>) {
        store.clear();
    }

    // ------------------------------------------------------------- cookie path

    fn session_from_environment(&self) -> Option<LinkedInSession> {
        let settings = &self.settings;
        let mut cookies: HashMap<String, String> = HashMap::new();

        if let Some(header) = settings.cookie_header() {
            cookies.extend(super::session::parse_cookie_header(header));
        }
        if let Some(value) = settings.li_at() {
            cookies.insert(
                LI_AT.to_string(),
                value.trim().trim_matches('"').to_string(),
            );
        }
        if let Some(value) = settings.jsessionid() {
            cookies.insert(JSESSIONID.to_string(), quote_cookie(value));
        }
        for (name, value) in settings.extra_cookies() {
            cookies.insert(name.clone(), value.clone());
        }

        if !cookies.contains_key(LI_AT) {
            return None;
        }
        tracing::info!("auth.using_environment_cookie");
        Some(LinkedInSession {
            cookies,
            source: SessionSource::Environment,
            ..Default::default()
        })
    }

    /// Fetch a `JSESSIONID` when the operator supplied only `li_at`.
    async fn ensure_csrf_token(
        &self,
        client: &Client,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        session: &mut LinkedInSession,
    ) -> Result<(), crate::error::AppError> {
        if session.csrf_token().is_some() {
            return Ok(());
        }
        self.apply_session(jar, session);
        let response = client
            .get(format!(
                "{}{}",
                endpoints::BASE_URL,
                endpoints::AUTH_SEED_URL
            ))
            .headers(self.browser_headers())
            .send()
            .await
            .map_err(|e| {
                lerr::auth_failed(&format!("Could not reach LinkedIn: {e}"), "auth-seed")
            })?;
        let jsessionid = self
            .find_cookie(jar, JSESSIONID)
            .or_else(|| set_cookie_value(response.headers(), JSESSIONID));
        let jsessionid = match jsessionid {
            Some(value) => value,
            None => {
                return Err(lerr::auth_failed(
                    "LinkedIn did not issue a JSESSIONID; set LINKEDIN_JSESSIONID explicitly.",
                    "auth-seed",
                ));
            }
        };
        session.merge_cookies(&HashMap::from([(
            JSESSIONID.to_string(),
            quote_cookie(&jsessionid),
        )]));
        tracing::info!("auth.csrf_token_seeded");
        Ok(())
    }

    /// Replace the CSRF token with one LinkedIn issues for this auth cookie.
    async fn reseed_csrf_token(
        &self,
        client: &Client,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        session: &mut LinkedInSession,
    ) -> bool {
        let previous = session.jsessionid().map(str::to_string);
        session.cookies.remove(JSESSIONID);
        let result = self.ensure_csrf_token(client, jar, session).await;
        match result {
            Ok(_) => session.jsessionid().is_some() && session.jsessionid() != previous.as_deref(),
            Err(_) => {
                if let Some(previous) = previous {
                    session.cookies.insert(JSESSIONID.to_string(), previous);
                }
                false
            }
        }
    }

    // ----------------------------------------------------------- password path

    async fn login_with_credentials(
        &self,
        client: &Client,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
    ) -> Result<LinkedInSession, crate::error::AppError> {
        let empty = LinkedInSession::default();
        self.apply_session(jar, &empty);
        let seed = client
            .get(format!(
                "{}{}",
                endpoints::BASE_URL,
                endpoints::AUTH_SEED_URL
            ))
            .headers(self.browser_headers())
            .send()
            .await
            .map_err(|e| lerr::auth_failed(&format!("LinkedIn is unreachable: {e}"), "login"))?;
        let jsessionid = self
            .find_cookie(jar, JSESSIONID)
            .or_else(|| set_cookie_value(seed.headers(), JSESSIONID));
        let jsessionid = match jsessionid {
            Some(value) => quote_cookie(&value),
            None => {
                return Err(lerr::auth_failed(
                    "LinkedIn did not return a login seed cookie.",
                    "login",
                ));
            }
        };

        tracing::info!("auth.password_login_start");
        let mut headers = self.browser_headers();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert(
            "csrf-token",
            HeaderValue::from_str(jsessionid.trim_matches('"')).expect("plain latin token"),
        );
        headers.insert("x-li-user-agent", HeaderValue::from_static(LI_USER_AGENT));
        headers.insert(
            "origin",
            HeaderValue::from_static("https://www.linkedin.com"),
        );
        headers.insert(
            "referer",
            HeaderValue::from_static("https://www.linkedin.com/login"),
        );

        let mut form = vec![
            (
                "session_key".to_string(),
                self.settings.email().unwrap_or_default().to_string(),
            ),
            (
                "session_password".to_string(),
                self.settings.password().unwrap_or_default().to_string(),
            ),
            ("JSESSIONID".to_string(), jsessionid.clone()),
        ];
        form.retain(|(_, v)| !v.is_empty());
        let response = client
            .post(format!(
                "{}{}",
                endpoints::BASE_URL,
                endpoints::AUTH_SUBMIT_URL
            ))
            .headers(headers)
            .form(&form)
            .send()
            .await
            .map_err(|e| lerr::auth_failed(&format!("LinkedIn is unreachable: {e}"), "login"))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(lerr::auth_failed(
                "LinkedIn rejected the supplied e-mail or password.",
                "login",
            ));
        }
        if response.status().is_client_error() {
            return Err(lerr::auth_failed(
                "Unexpected response from the LinkedIn login endpoint.",
                "login",
            ));
        }

        let login_li_at = set_cookie_value(response.headers(), LI_AT);
        let payload = safe_json(response).await;
        let result = payload
            .get("login_result")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_uppercase();
        if !result.is_empty() && result != "PASS" {
            return Err(challenge_or_bad_credentials(&result));
        }

        let li_at = self.find_cookie(jar, LI_AT).or(login_li_at);
        let li_at = match li_at {
            Some(value) if !value.is_empty() => value,
            _ => {
                return Err(lerr::auth_failed(
                    "Login did not yield an li_at cookie.",
                    "login",
                ));
            }
        };
        let final_jsessionid = self
            .find_cookie(jar, JSESSIONID)
            .map(|v| quote_cookie(&v))
            .unwrap_or(jsessionid);
        tracing::info!("auth.password_login_success");
        Ok(LinkedInSession {
            cookies: HashMap::from([
                (LI_AT.to_string(), li_at),
                (JSESSIONID.to_string(), final_jsessionid),
            ]),
            source: SessionSource::PasswordLogin,
            ..Default::default()
        })
    }

    fn find_cookie(
        &self,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        name: &str,
    ) -> Option<String> {
        let store = jar.lock().unwrap();
        store
            .get_request_values(&jar_url())
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.to_string())
    }

    // -------------------------------------------------------------- validation

    /// Confirm the session is live using one JSON API call, never a page.
    pub async fn validate(
        &self,
        client: &Client,
        jar: &Arc<reqwest_cookie_store::CookieStoreMutex>,
        session: &mut LinkedInSession,
    ) -> Result<bool, crate::error::AppError> {
        self.apply_session(jar, session);
        let response = client
            .get(format!("{}{}", endpoints::BASE_URL, endpoints::ME))
            .headers(self.voyager_headers(session, &format!("{}/feed/", endpoints::BASE_URL)))
            .send()
            .await
            .map_err(|e| lerr::auth_failed(&format!("Could not reach LinkedIn: {e}"), "me"))?;
        self.absorb_cookies(jar, session);

        let status = response.status();

        if status == reqwest::StatusCode::OK {
            let payload = safe_json(response).await;
            let mini = payload.get("miniProfile").and_then(Value::as_object);
            session.member_urn = mini
                .and_then(|m| m.get("entityUrn"))
                .or_else(|| payload.get("entityUrn"))
                .and_then(Value::as_str)
                .map(str::to_string);
            session.last_validated_at = Some(std::time::SystemTime::now());
            tracing::info!(source = session.source.as_str(), "auth.session_valid");
            return Ok(true);
        }

        // Capture the failure body: empty/HTML means an anti-bot block by
        // source IP, "CSRF check failed." means a cookie pair problem. This
        // is what tells cloud operators apart from a bad env var.
        let sign_out = session.signed_out() || is_sign_out(&response);
        let response_body = response.text().await.unwrap_or_default();
        let body_snippet = &response_body[..response_body.len().min(256)];

        if sign_out {
            tracing::warn!(status = ?status, "auth.signed_out_by_linkedin");
            return Err(lerr::session_expired(
                "LinkedIn signed this session out server-side. Capture a fresh cookie set \
                 from a signed-in browser - ideally the entire Cookie header, so the \
                 device cookies (bcookie, bscookie, liap) travel with the auth token - \
                 and set USER_AGENT to that browser's user agent.",
                "me",
                Some(i64::from(status.as_u16())),
            ));
        }

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::warn!(status = ?status, body = %body_snippet, "auth.session_invalid");
            return Ok(false);
        }
        // Any other status (400 = CSRF mismatch, 429, 5xx, ...) fails closed:
        // a "maybe valid" session is what lets a stale on-disk session shadow a
        // fresh environment cookie set and degrades to PROFILE_NOT_VISIBLE.
        tracing::warn!(status = ?status, body = %body_snippet, "auth.validation_inconclusive");
        Ok(false)
    }

    // ----------------------------------------------------------------- headers

    pub fn browser_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            header_value(&self.settings.user_agent),
        );
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            header_value(&self.settings.accept_language),
        );
        headers.insert("x-user-language", HeaderValue::from_static("en"));
        headers.insert("x-user-locale", HeaderValue::from_static("en_US"));
        headers.insert(
            reqwest::header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        );
        headers.insert(
            reqwest::header::PRAGMA,
            HeaderValue::from_static("no-cache"),
        );
        headers
    }

    pub fn voyager_headers(&self, session: &LinkedInSession, referer: &str) -> HeaderMap {
        let mut headers = self.browser_headers();
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "csrf-token",
            header_value(session.csrf_token().as_deref().unwrap_or("")),
        );
        headers.insert(
            "x-restli-protocol-version",
            HeaderValue::from_static("2.0.0"),
        );
        headers.insert("x-li-lang", HeaderValue::from_static("en_US"));
        headers.insert("x-li-track", header_value(&li_track(&self.settings)));
        headers.insert("x-li-user-agent", HeaderValue::from_static(LI_USER_AGENT));
        headers.insert(reqwest::header::REFERER, header_value(referer));
        headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
        headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        headers
    }

    /// Normalised accept header for GraphQL payloads.
    pub fn normalized_accept(&self) -> HeaderValue {
        HeaderValue::from_static("application/vnd.linkedin.normalized+json+2.1")
    }

    pub fn li_page_instance(&self) -> String {
        format!(
            "urn:li:page:d_flagship3_profile_view_base;{}",
            uuid::Uuid::new_v4()
        )
    }
}

fn jar_url() -> reqwest::Url {
    reqwest::Url::parse(&format!("https://{COOKIE_DOMAIN}/")).expect("canned url")
}

fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn li_track(settings: &Settings) -> String {
    format!(
        concat!(
            r#"{{"clientVersion":"{}","mpVersion":"{}","osName":"web","timezoneOffset":0,"#,
            r#""timezone":"UTC","deviceFormFactor":"DESKTOP","mpName":"voyager-web","#,
            r#""displayDensity":1,"displayWidth":1920,"displayHeight":1080}}"#
        ),
        settings.voyager_client_version, settings.voyager_client_version
    )
}

async fn safe_json(response: reqwest::Response) -> serde_json::Value {
    response
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null)
}

fn set_cookie_value(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(reqwest::header::SET_COOKIE) {
        let Ok(value) = value.to_str() else { continue };
        let pair = value.split(';').next()?;
        let (cname, cvalue) = pair.split_once('=')?;
        if cname.trim().eq_ignore_ascii_case(name) {
            return Some(cvalue.trim().to_string());
        }
    }
    None
}

fn challenge_or_bad_credentials(result: &str) -> crate::error::AppError {
    match result {
        "CHALLENGE" | "PASS_CAPTCHA" => lerr::challenge_required("login"),
        _ => lerr::auth_failed("LinkedIn rejected the supplied credentials.", "login"),
    }
}

/// A self-redirect that expires the auth cookie is LinkedIn logging us out.
fn is_sign_out(response: &reqwest::Response) -> bool {
    if !response.status().is_redirection() {
        return false;
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim_end_matches('/');
    let url = response.url().as_str().trim_end_matches('/');
    let set_cookie_bears_li_at = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|value| value.contains(LI_AT));
    location == url || set_cookie_bears_li_at
}
