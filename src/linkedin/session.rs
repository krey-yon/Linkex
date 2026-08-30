//! LinkedIn session representation and on-disk persistence.
//!
//! A session is nothing more than the cookie jar LinkedIn's web client keeps:
//! `li_at` (the member auth token) and `JSESSIONID` (whose value doubles as the
//! CSRF token every Voyager call echoes back). Persisting it means a restart
//! does not trigger another login, which is the fastest way to get an account
//! flagged.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use thiserror::Error;

pub const LI_AT: &str = "li_at";
pub const JSESSIONID: &str = "JSESSIONID";
pub const TOMBSTONE: &str = "delete me";

/// Device/routing cookies that keep a li_at alive; replayed from the browser
/// cookie set as configured by the operator.
pub const BROWSER_COOKIES: &[&str] = &[
    LI_AT,
    JSESSIONID,
    "li_rm",
    "li_a",
    "liap",
    "li_gc",
    "li_mc",
    "li_theme",
    "li_theme_set",
    "bcookie",
    "bscookie",
    "lidc",
    "lang",
    "timezone",
    "dfpfpt",
    "AnalyticsSyncHistory",
    "UserMatchHistory",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSource {
    Environment,
    PasswordLogin,
    Restored,
}

impl SessionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionSource::Environment => "environment",
            SessionSource::PasswordLogin => "password-login",
            SessionSource::Restored => "restored",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LinkedInSession {
    pub cookies: HashMap<String, String>,
    pub source: SessionSource,
    pub created_at: SystemTime,
    pub last_validated_at: Option<SystemTime>,
    pub member_urn: Option<String>,
}

impl Default for LinkedInSession {
    fn default() -> Self {
        Self {
            cookies: HashMap::new(),
            source: SessionSource::Environment,
            created_at: SystemTime::now(),
            last_validated_at: None,
            member_urn: None,
        }
    }
}

/// LinkedIn expires a cookie by setting this literal value.
pub fn is_tombstone(value: &str) -> bool {
    let stripped = value.trim().trim_matches('"');
    stripped == TOMBSTONE
}

impl LinkedInSession {
    pub fn li_at(&self) -> Option<&str> {
        self.cookies.get(LI_AT).map(String::as_str)
    }

    pub fn jsessionid(&self) -> Option<&str> {
        self.cookies.get(JSESSIONID).map(String::as_str)
    }

    /// The CSRF token is the JSESSIONID value with its literal quotes removed.
    pub fn csrf_token(&self) -> Option<String> {
        self.jsessionid().map(|v| v.trim_matches('"').to_string())
    }

    pub fn is_usable(&self) -> bool {
        self.li_at().is_some()
            && self.csrf_token().is_some()
            && !self.li_at().is_some_and(is_tombstone)
    }

    /// True once LinkedIn has expired the auth cookie server-side.
    pub fn signed_out(&self) -> bool {
        self.li_at().is_some_and(is_tombstone)
    }

    pub fn age_seconds(&self) -> f64 {
        self.created_at
            .elapsed()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    /// Fold newly issued cookies in, honouring deletion instructions.
    pub fn merge_cookies(&mut self, cookies: &HashMap<String, String>) {
        for (name, value) in cookies {
            if value.is_empty() {
                continue;
            }
            if is_tombstone(value) {
                self.cookies.remove(name);
                if name == LI_AT {
                    self.cookies.insert(LI_AT.to_string(), value.clone());
                }
                continue;
            }
            self.cookies.insert(name.clone(), value.clone());
        }
    }

    /// Session metadata safe to expose over the API.
    pub fn public_state(&self) -> Value {
        // A session whose auth token is the tombstone is signed out; report it
        // as unauthenticated rather than dead.
        let cookie_names: Vec<String> = self.cookies.keys().cloned().collect();
        let mut names: Vec<&str> = cookie_names.iter().map(String::as_str).collect();
        names.sort_unstable();
        json!({
            "authenticated": !self.signed_out() && self.is_usable(),
            "source": self.source.as_str(),
            "age_seconds": (self.age_seconds() * 10.0).round() / 10.0,
            "last_validated_at": self.last_validated_at.map(to_epoch_seconds),
            "member_urn": self.member_urn,
            "cookie_names": names,
        })
    }
}

fn to_epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// --------------------------------------------------------------------- store

/// Persistence seam: tests use an in-memory store.
pub trait SessionStore: Send + Sync + 'static {
    fn load(&self) -> Option<LinkedInSession>;
    fn save(&self, session: &LinkedInSession);
    fn clear(&self);
    fn enabled(&self) -> bool;
}

impl SessionStore for Arc<InMemoryStore> {
    fn load(&self) -> Option<LinkedInSession> {
        self.inner.read().unwrap().clone()
    }
    fn save(&self, session: &LinkedInSession) {
        *self.inner.write().unwrap() = Some(session.clone());
    }
    fn clear(&self) {
        *self.inner.write().unwrap() = None;
    }
    fn enabled(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct InMemoryStore {
    inner: std::sync::RwLock<Option<LinkedInSession>>,
}

impl InMemoryStore {
    pub fn shared() -> Arc<InMemoryStore> {
        Arc::new(InMemoryStore::default())
    }
}

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("failed to serialise session: {0}")]
    Serialize(String),
    #[error("failed to write session state: {0}")]
    Write(#[from] std::io::Error),
}

/// Atomic JSON persistence for a `LinkedInSession`.
pub struct FileSessionStore {
    path: Option<PathBuf>,
}

impl FileSessionStore {
    pub fn new(path: &str) -> Self {
        let path = if path.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        };
        FileSessionStore { path }
    }

    fn save_sync(path: &Path, session: &LinkedInSession) -> Result<(), PersistenceError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let payload = json!({
            "cookies": session.cookies,
            "source": session.source.as_str(),
            "created_at": to_epoch_seconds(session.created_at),
            "last_validated_at": session.last_validated_at.map(to_epoch_seconds),
            "member_urn": session.member_urn,
        });

        let mut temp = tempfile_in_parent(path)?;
        serde_json::to_writer_pretty(&mut temp, &payload)
            .map_err(|e| PersistenceError::Serialize(e.to_string()))?;
        temp.write_all(b"\n")?;
        temp.flush()?;
        let temp_path = temp_path_of(path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600));
        }
        drop(temp);
        std::fs::rename(&temp_path, path)?;
        Ok(())
    }

    fn load_sync(path: &Path) -> Result<LinkedInSession, PersistenceError> {
        let raw = std::fs::read_to_string(path)?;
        let payload: Value =
            serde_json::from_str(&raw).map_err(|e| PersistenceError::Serialize(e.to_string()))?;
        let cookies_map = payload
            .get("cookies")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut cookies = HashMap::new();
        for (name, value) in cookies_map {
            if let Some(value) = value.as_str() {
                cookies.insert(name, value.to_string());
            }
        }
        if !cookies.contains_key(LI_AT) {
            // Malformed state: quarantine and report failure.
            let _ = std::fs::remove_file(path);
            return Err(PersistenceError::Serialize(
                "session state has no li_at cookie".to_string(),
            ));
        }
        let created = payload
            .get("created_at")
            .and_then(Value::as_f64)
            .map(epoch_to_system_time)
            .unwrap_or_else(SystemTime::now);
        let validated = payload
            .get("last_validated_at")
            .and_then(Value::as_f64)
            .map(epoch_to_system_time);
        let member_urn = payload
            .get("member_urn")
            .and_then(Value::as_str)
            .map(str::to_string);

        Ok(LinkedInSession {
            cookies,
            source: SessionSource::Restored,
            created_at: created,
            last_validated_at: validated,
            member_urn,
        })
    }
}

fn tempfile_in_parent(path: &Path) -> Result<std::fs::File, std::io::Error> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    std::fs::File::create(temp_path_of(path))
}

fn temp_path_of(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    );
    dir.join(name)
}

fn epoch_to_system_time(seconds: f64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_secs_f64(seconds.max(0.0))
}

impl SessionStore for FileSessionStore {
    fn load(&self) -> Option<LinkedInSession> {
        let path = self.path.as_ref()?;
        match FileSessionStore::load_sync(path) {
            Ok(session) => Some(session),
            Err(PersistenceError::Write(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                tracing::warn!(error = %err, "session.restore_failed");
                // Malformed state must not be retried forever; quarantine it.
                let _ = std::fs::remove_file(path);
                None
            }
        }
    }

    fn save(&self, session: &LinkedInSession) {
        if let Some(path) = self.path.as_ref()
            && let Err(err) = FileSessionStore::save_sync(path, session)
        {
            tracing::warn!(error = %err, "session.persist_failed");
        }
    }

    fn clear(&self) {
        if let Some(path) = self.path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
    }

    fn enabled(&self) -> bool {
        self.path.is_some()
    }
}

// ------------------------------------------------------------ cookie parsing

/// Parse a browser `Cookie:` header into a mapping. Every pair is kept
/// verbatim; the header itself is never logged.
pub fn parse_cookie_header(header: &str) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    for chunk in header.split(';') {
        let Some((name, value)) = chunk.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if !name.is_empty() {
            cookies.insert(name.to_string(), value.to_string());
        }
    }
    cookies
}

/// Wrap a value in literal quotes if not already quoted (JSESSIONID style).
pub fn quote_cookie(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"') {
        value.to_string()
    } else {
        format!("\"{value}\"")
    }
}

#[allow(dead_code)]
pub fn browser_cookie_names() -> HashSet<&'static str> {
    BROWSER_COOKIES.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_token_strips_quotes() {
        let mut session = LinkedInSession::default();
        session.cookies.insert(LI_AT.into(), "abc".into());
        session.cookies.insert(JSESSIONID.into(), "\"xyz\"".into());
        assert_eq!(session.csrf_token().as_deref(), Some("xyz"));
        session.cookies.insert(JSESSIONID.into(), "xyz".into());
        assert_eq!(session.csrf_token().as_deref(), Some("xyz"));
        assert!(session.is_usable());
    }

    #[test]
    fn tombstone_detection() {
        assert!(is_tombstone("delete me"));
        assert!(is_tombstone(" delete me "));
        assert!(is_tombstone("\"delete me\""));
        assert!(!is_tombstone("delete-me"));
        assert!(!is_tombstone(""));
        let mut session = LinkedInSession::default();
        session.cookies.insert(LI_AT.into(), "\"delete me\"".into());
        assert!(session.signed_out());
        assert!(!session.is_usable());
    }

    #[test]
    fn cookie_header_parsing() {
        let cookies = parse_cookie_header("li_at=abc; JSESSIONID=\"xyz\"; bcookie=v1:1;");
        assert_eq!(cookies.get("li_at").map(String::as_str), Some("abc"));
        assert_eq!(
            cookies.get("JSESSIONID").map(String::as_str),
            Some("\"xyz\"")
        );
        assert_eq!(cookies.get("bcookie").map(String::as_str), Some("v1:1"));
        assert!(parse_cookie_header("no equals here").is_empty());
    }

    #[test]
    fn merge_rotations_honours_deletions() {
        let mut session = LinkedInSession::default();
        session.cookies.insert(LI_AT.into(), "token".into());
        let rotated = HashMap::from([
            ("lidc".to_string(), "b=..:t=..".to_string()),
            (LI_AT.to_string(), "delete me".to_string()),
        ]);
        session.merge_cookies(&rotated);
        assert!(session.signed_out());
        assert_eq!(
            session.cookies.get("lidc").map(String::as_str),
            Some("b=..:t=..")
        );
        let revived = HashMap::from([(LI_AT.to_string(), "token2".to_string())]);
        session.merge_cookies(&revived);
        assert!(!session.signed_out());
        assert_eq!(session.li_at(), Some("token2"));
    }

    #[test]
    fn public_state_is_redacted() {
        let mut session = LinkedInSession::default();
        session.cookies.insert(LI_AT.into(), "super-secret".into());
        session.cookies.insert(JSESSIONID.into(), "\"csrf\"".into());
        session.cookies.insert("bcookie".into(), "v=1".into());
        let state = session.public_state();
        assert_eq!(state["authenticated"], true);
        assert_eq!(state["source"], "environment");
        assert!(
            state["cookie_names"]
                .as_array()
                .unwrap()
                .contains(&Value::String("li_at".into()))
        );
        assert!(state.get("cookies").is_none());
        assert!(!state.to_string().contains("super-secret"));
    }

    #[test]
    fn file_store_round_trip_and_atomicity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let store = FileSessionStore::new(path.to_str().unwrap());
        let mut session = LinkedInSession::default();
        session.cookies.insert(LI_AT.into(), "token".into());
        session
            .cookies
            .insert(JSESSIONID.into(), "\"csrftoken\"".into());
        session.member_urn = Some("urn:li:member:1".into());
        store.save(&session);
        store.save(&session); // second write must not corrupt

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("csrftoken"));
        let restored = store.load().unwrap();
        assert_eq!(restored.li_at(), Some("token"));
        assert_eq!(restored.member_urn.as_deref(), Some("urn:li:member:1"));
        assert_eq!(restored.source, SessionSource::Restored);
        assert_eq!(restored.csrf_token().as_deref(), Some("csrftoken"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        store.clear();
        assert!(store.load().is_none());
    }

    #[test]
    fn malformed_state_is_quarantined_not_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, "{not json").unwrap();
        let store = FileSessionStore::new(path.to_str().unwrap());
        assert!(store.load().is_none());

        std::fs::write(&path, r#"{"cookies": {"bcookie": "x"}}"#).unwrap();
        assert!(store.load().is_none());

        assert!(FileSessionStore::new("").load().is_none());
        assert!(!FileSessionStore::new("").enabled());
    }

    #[test]
    fn quote_helpers() {
        assert_eq!(quote_cookie("abc"), "\"abc\"");
        assert_eq!(quote_cookie("\"abc\""), "\"abc\"");
        assert_eq!(quote_cookie(" x ' "), "\"x '\"");
    }
}
