//! Immutable application settings loaded once at startup.
//!
//! `Settings::from_env()` validates values and fails startup on malformed or
//! unsafe configuration. Secrets are redacted from `Debug` output and never
//! appear in `ConfigError` text.

use std::env;
use std::time::Duration;

use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid value for {name}: {reason}")]
    Invalid { name: String, reason: String },
}

const LOG_LEVELS: &[&str] = &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

/// Env var -> cookie name for the device/routing cookies that keep `li_at`
/// alive (as set in a signed-in browser).
const EXTRA_COOKIE_ENV_MAP: &[(&str, &str)] = &[
    ("LINKEDIN_BCOOKIE", "bcookie"),
    ("LINKEDIN_B_SCOOKIE", "bscookie"),
    ("LINKEDIN_LIAP", "liap"),
    ("LINKEDIN_LIDC", "lidc"),
];

#[derive(Clone)]
pub struct Settings {
    // ------------------------------------------------------------------ service
    pub app_name: String,
    pub app_version: String,
    pub environment: Environment,
    pub log_level: String,
    pub log_format: LogFormat,
    pub host: String,
    pub port: u16,
    pub root_path: String,

    // ---------------------------------------------------------------- our API
    pub api_keys: Vec<String>,
    pub billing_enabled: bool,
    pub redis_url: String,
    pub api_key_seed_path: String,
    pub cache_hit_cost_cents: i64,
    pub cache_miss_cost_cents: i64,
    pub cors_origins: Vec<String>,
    pub rate_limit_requests: u64,
    pub rate_limit_window_seconds: u64,
    pub expose_raw_endpoint: bool,
    pub max_request_body_bytes: usize,
    pub trusted_proxies: Vec<String>,

    // ------------------------------------------------------- linkedin identity
    linkedin_li_at: Option<String>,
    linkedin_jsessionid: Option<String>,
    linkedin_extra_cookies: Vec<(String, String)>,
    linkedin_email: Option<String>,
    linkedin_password: Option<String>,
    linkedin_cookie_header: Option<String>,
    pub allow_password_login: bool,
    linkedin_profile_query_id: Option<String>,
    pub session_state_path: String,
    cookie_fingerprint: Option<String>,

    // ---------------------------------------------------------- upstream client
    pub user_agent: String,
    pub accept_language: String,
    pub voyager_client_version: String,
    pub request_timeout: Duration,
    pub max_retries: usize,
    pub retry_backoff_seconds: f64,
    pub upstream_min_interval_seconds: f64,
    pub upstream_jitter_seconds: f64,
    pub upstream_max_concurrency: usize,
    pub circuit_breaker_threshold: usize,
    pub circuit_breaker_cooldown_seconds: f64,
    proxy_url: Option<String>,

    // ------------------------------------------------------------------- cache
    pub cache_ttl_seconds: u64,
    pub cache_max_entries: usize,
    pub cache_dir: String,
    pub cache_persist: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            app_name: "LinkedIn Profile API".to_string(),
            app_version: "0.1.0".to_string(),
            environment: Environment::Development,
            log_level: "INFO".to_string(),
            log_format: LogFormat::Console,
            host: "0.0.0.0".to_string(),
            port: 8000,
            root_path: String::new(),
            api_keys: Vec::new(),
            billing_enabled: false,
            redis_url: "redis://127.0.0.1:6379/".to_string(),
            api_key_seed_path: "config/api_keys.json".to_string(),
            cache_hit_cost_cents: 25,
            cache_miss_cost_cents: 50,
            cors_origins: vec!["*".to_string()],
            rate_limit_requests: 30,
            rate_limit_window_seconds: 60,
            expose_raw_endpoint: false,
            max_request_body_bytes: 16 * 1024,
            trusted_proxies: Vec::new(),
            linkedin_li_at: None,
            linkedin_jsessionid: None,
            linkedin_extra_cookies: Vec::new(),
            linkedin_email: None,
            linkedin_password: None,
            linkedin_cookie_header: None,
            allow_password_login: false,
            linkedin_profile_query_id: None,
            session_state_path: ".cache/linkedin_session.json".to_string(),
            cookie_fingerprint: None,
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, \
                         like Gecko) Chrome/126.0.0.0 Safari/537.36"
                .to_string(),
            accept_language: "en-US,en;q=0.9".to_string(),
            voyager_client_version: "1.13.30626".to_string(),
            request_timeout: Duration::from_secs_f64(20.0),
            max_retries: 3,
            retry_backoff_seconds: 1.5,
            upstream_min_interval_seconds: 1.2,
            upstream_jitter_seconds: 0.6,
            upstream_max_concurrency: 4,
            circuit_breaker_threshold: 4,
            circuit_breaker_cooldown_seconds: 90.0,
            proxy_url: None,
            cache_ttl_seconds: 86_400,
            cache_max_entries: 512,
            cache_dir: ".cache/profiles".to_string(),
            cache_persist: true,
        }
    }
}

/// Hand-written Debug: secret values are never printed.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("app_name", &self.app_name)
            .field("app_version", &self.app_version)
            .field("environment", &self.environment)
            .field("log_level", &self.log_level)
            .field("log_format", &self.log_format)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("root_path", &self.root_path)
            .field("api_key_count", &self.api_keys.len())
            .field("billing_enabled", &self.billing_enabled)
            .field("api_key_seed_path", &self.api_key_seed_path)
            .field("cache_hit_cost_cents", &self.cache_hit_cost_cents)
            .field("cache_miss_cost_cents", &self.cache_miss_cost_cents)
            .field("cors_origins", &self.cors_origins)
            .field("rate_limit_requests", &self.rate_limit_requests)
            .field("rate_limit_window_seconds", &self.rate_limit_window_seconds)
            .field("expose_raw_endpoint", &self.expose_raw_endpoint)
            .field("max_request_body_bytes", &self.max_request_body_bytes)
            .field("trusted_proxies", &self.trusted_proxies)
            .field("has_li_at", &self.linkedin_li_at.is_some())
            .field("has_cookie_header", &self.linkedin_cookie_header.is_some())
            .field("cookie_fingerprint", &self.cookie_fingerprint)
            .field("has_jsessionid", &self.linkedin_jsessionid.is_some())
            .field(
                "extra_cookies",
                &self
                    .linkedin_extra_cookies
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("has_email", &self.linkedin_email.is_some())
            .field("has_password", &self.linkedin_password.is_some())
            .field("allow_password_login", &self.allow_password_login)
            .field(
                "has_profile_query_id",
                &self.linkedin_profile_query_id.is_some(),
            )
            .field("session_state_path", &self.session_state_path)
            .field("user_agent", &self.user_agent)
            .field("accept_language", &self.accept_language)
            .field("voyager_client_version", &self.voyager_client_version)
            .field("request_timeout", &self.request_timeout)
            .field("max_retries", &self.max_retries)
            .field("retry_backoff_seconds", &self.retry_backoff_seconds)
            .field(
                "upstream_min_interval_seconds",
                &self.upstream_min_interval_seconds,
            )
            .field("upstream_jitter_seconds", &self.upstream_jitter_seconds)
            .field("upstream_max_concurrency", &self.upstream_max_concurrency)
            .field("circuit_breaker_threshold", &self.circuit_breaker_threshold)
            .field(
                "circuit_breaker_cooldown_seconds",
                &self.circuit_breaker_cooldown_seconds,
            )
            .field("proxy_configured", &self.proxy_url.is_some())
            .field("cache_ttl_seconds", &self.cache_ttl_seconds)
            .field("cache_max_entries", &self.cache_max_entries)
            .field("cache_dir", &self.cache_dir)
            .field("cache_persist", &self.cache_persist)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Development => "development",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }

    fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.to_lowercase().as_str() {
            "development" => Ok(Environment::Development),
            "staging" => Ok(Environment::Staging),
            "production" => Ok(Environment::Production),
            other => Err(ConfigError::Invalid {
                name: "ENVIRONMENT".to_string(),
                reason: format!("expected development|staging|production, got '{other}'"),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Console,
    Json,
}

impl LogFormat {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.to_lowercase().as_str() {
            "console" => Ok(LogFormat::Console),
            "json" => Ok(LogFormat::Json),
            other => Err(ConfigError::Invalid {
                name: "LOG_FORMAT".to_string(),
                reason: format!("expected console|json, got '{other}'"),
            }),
        }
    }
}

impl Settings {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut s = Settings::default();

        // ----------------------------------------------------------------- service
        if let Some(value) = nonempty_var("APP_NAME") {
            s.app_name = value;
        }
        if let Some(value) = nonempty_var("APP_VERSION") {
            s.app_version = value;
        }
        if let Some(value) = nonempty_var("ENVIRONMENT") {
            s.environment = Environment::parse(&value)?;
        }
        if let Some(value) = nonempty_var("LOG_LEVEL") {
            let upper = value.to_uppercase();
            if !LOG_LEVELS.contains(&upper.as_str()) {
                return Err(ConfigError::Invalid {
                    name: "LOG_LEVEL".to_string(),
                    reason: format!("'{value}' is not a known level"),
                });
            }
            s.log_level = upper;
        }
        if let Some(value) = nonempty_var("LOG_FORMAT") {
            s.log_format = LogFormat::parse(&value)?;
        }
        if let Some(value) = nonempty_var("HOST") {
            s.host = value;
        }
        if let Some(value) = nonempty_var("PORT") {
            let port: u16 = value.parse().map_err(|_| ConfigError::Invalid {
                name: "PORT".to_string(),
                reason: format!("'{value}' is not a valid port"),
            })?;
            if port == 0 {
                return Err(ConfigError::Invalid {
                    name: "PORT".to_string(),
                    reason: "port must be nonzero".to_string(),
                });
            }
            s.port = port;
        }
        if let Some(value) = nonempty_var("ROOT_PATH") {
            s.root_path = value;
        }

        // -------------------------------------------------------------- our API
        s.api_keys = parse_csv("API_KEYS");
        if let Some(value) = nonempty_var("BILLING_ENABLED") {
            s.billing_enabled = parse_bool("BILLING_ENABLED", &value)?;
        }
        if let Some(value) = nonempty_var("REDIS_URL") {
            s.redis_url = value;
        }
        if let Some(value) = nonempty_var("API_KEY_SEED_PATH") {
            s.api_key_seed_path = value;
        }
        if let Some(value) = nonempty_var("CACHE_HIT_COST_CENTS") {
            s.cache_hit_cost_cents = parse_i64("CACHE_HIT_COST_CENTS", &value)?;
        }
        if let Some(value) = nonempty_var("CACHE_MISS_COST_CENTS") {
            s.cache_miss_cost_cents = parse_i64("CACHE_MISS_COST_CENTS", &value)?;
        }
        if s.cache_hit_cost_cents < 0 || s.cache_miss_cost_cents < s.cache_hit_cost_cents {
            return Err(ConfigError::Invalid {
                name: "CACHE_*_COST_CENTS".to_string(),
                reason: "costs must be nonnegative and miss cost must be at least hit cost"
                    .to_string(),
            });
        }
        if let Some(value) = nonempty_var("CORS_ORIGINS") {
            s.cors_origins = split_csv(&value);
        }
        if let Some(value) = nonempty_var("RATE_LIMIT_REQUESTS") {
            s.rate_limit_requests = parse_u64("RATE_LIMIT_REQUESTS", &value)?;
            if s.rate_limit_requests == 0 {
                return Err(ConfigError::Invalid {
                    name: "RATE_LIMIT_REQUESTS".to_string(),
                    reason: "must be at least 1".to_string(),
                });
            }
        }
        if let Some(value) = nonempty_var("RATE_LIMIT_WINDOW_SECONDS") {
            s.rate_limit_window_seconds = parse_u64("RATE_LIMIT_WINDOW_SECONDS", &value)?;
            if s.rate_limit_window_seconds == 0 {
                return Err(ConfigError::Invalid {
                    name: "RATE_LIMIT_WINDOW_SECONDS".to_string(),
                    reason: "must be at least 1".to_string(),
                });
            }
        }
        if let Some(value) = nonempty_var("EXPOSE_RAW_ENDPOINT") {
            s.expose_raw_endpoint = parse_bool("EXPOSE_RAW_ENDPOINT", &value)?;
        }
        if let Some(value) = nonempty_var("MAX_REQUEST_BODY_BYTES") {
            s.max_request_body_bytes = parse_usize("MAX_REQUEST_BODY_BYTES", &value)?;
            if !(1..=1_048_576).contains(&s.max_request_body_bytes) {
                return Err(ConfigError::Invalid {
                    name: "MAX_REQUEST_BODY_BYTES".to_string(),
                    reason: "must be between 1 and 1048576".to_string(),
                });
            }
        }
        s.trusted_proxies = parse_csv("TRUSTED_PROXIES");

        // ----------------------------------------------------- linkedin identity
        // Coolify's env UI escapes double quotes in pasted values (`"` ->
        // `\"`), which turns the cookie values into garbage LinkedIn 400s.
        // Strip those escapes on load; values must be kept quote-free anyway.
        s.linkedin_li_at = blank_to_none("LINKEDIN_LI_AT").map(|v| unescape_quotes(&v));
        s.linkedin_jsessionid = blank_to_none("LINKEDIN_JSESSIONID").map(|v| unescape_quotes(&v));
        for (env_name, cookie_name) in EXTRA_COOKIE_ENV_MAP {
            if let Some(value) = blank_to_none(env_name) {
                s.linkedin_extra_cookies
                    .push((cookie_name.to_string(), unescape_quotes(&value)));
            }
        }
        s.linkedin_email = blank_to_none("LINKEDIN_EMAIL");
        s.linkedin_password = blank_to_none("LINKEDIN_PASSWORD");
        s.linkedin_cookie_header =
            blank_to_none("LINKEDIN_COOKIE_HEADER").map(|v| unescape_quotes(&v));
        if let Some(value) = nonempty_var("ALLOW_PASSWORD_LOGIN") {
            s.allow_password_login = parse_bool("ALLOW_PASSWORD_LOGIN", &value)?;
        }
        s.linkedin_profile_query_id = blank_to_none("LINKEDIN_PROFILE_QUERY_ID");
        if let Some(value) = nonempty_var("SESSION_STATE_PATH") {
            s.session_state_path = value;
        }

        // ------------------------------------------------------- upstream client
        if let Some(value) = nonempty_var("USER_AGENT") {
            s.user_agent = value;
        }
        if let Some(value) = nonempty_var("ACCEPT_LANGUAGE") {
            s.accept_language = value;
        }
        if let Some(value) = nonempty_var("VOYAGER_CLIENT_VERSION") {
            s.voyager_client_version = value;
        }
        if let Some(value) = nonempty_var("REQUEST_TIMEOUT_SECONDS") {
            s.request_timeout =
                Duration::from_secs_f64(parse_f64("REQUEST_TIMEOUT_SECONDS", &value)?);
        }
        if let Some(value) = nonempty_var("MAX_RETRIES") {
            s.max_retries = parse_usize("MAX_RETRIES", &value)?;
            if s.max_retries > 10 {
                return Err(ConfigError::Invalid {
                    name: "MAX_RETRIES".to_string(),
                    reason: "must not exceed 10".to_string(),
                });
            }
        }
        if let Some(value) = nonempty_var("RETRY_BACKOFF_SECONDS") {
            s.retry_backoff_seconds = parse_f64("RETRY_BACKOFF_SECONDS", &value)?;
        }
        if let Some(value) = nonempty_var("UPSTREAM_MIN_INTERVAL_SECONDS") {
            s.upstream_min_interval_seconds = parse_f64("UPSTREAM_MIN_INTERVAL_SECONDS", &value)?;
        }
        if let Some(value) = nonempty_var("UPSTREAM_JITTER_SECONDS") {
            s.upstream_jitter_seconds = parse_f64("UPSTREAM_JITTER_SECONDS", &value)?;
        }
        if let Some(value) = nonempty_var("UPSTREAM_MAX_CONCURRENCY") {
            s.upstream_max_concurrency = parse_usize("UPSTREAM_MAX_CONCURRENCY", &value)?;
            if !(1..=64).contains(&s.upstream_max_concurrency) {
                return Err(ConfigError::Invalid {
                    name: "UPSTREAM_MAX_CONCURRENCY".to_string(),
                    reason: "must be between 1 and 64".to_string(),
                });
            }
        }
        if let Some(value) = nonempty_var("CIRCUIT_BREAKER_THRESHOLD") {
            s.circuit_breaker_threshold = parse_usize("CIRCUIT_BREAKER_THRESHOLD", &value)?;
            if !(1..=100).contains(&s.circuit_breaker_threshold) {
                return Err(ConfigError::Invalid {
                    name: "CIRCUIT_BREAKER_THRESHOLD".to_string(),
                    reason: "must be between 1 and 100".to_string(),
                });
            }
        }
        if let Some(value) = nonempty_var("CIRCUIT_BREAKER_COOLDOWN_SECONDS") {
            s.circuit_breaker_cooldown_seconds =
                parse_f64("CIRCUIT_BREAKER_COOLDOWN_SECONDS", &value)?;
        }
        if let Some(value) = blank_to_none("PROXY_URL") {
            s.proxy_url = Some(validate_proxy_url(&value)?);
        }

        // ------------------------------------------------------------------ cache
        if let Some(value) = nonempty_var("CACHE_TTL_SECONDS") {
            s.cache_ttl_seconds = parse_u64("CACHE_TTL_SECONDS", &value)?;
        }
        if let Some(value) = nonempty_var("CACHE_MAX_ENTRIES") {
            s.cache_max_entries = parse_usize("CACHE_MAX_ENTRIES", &value)?;
            if !(1..=1_000_000).contains(&s.cache_max_entries) {
                return Err(ConfigError::Invalid {
                    name: "CACHE_MAX_ENTRIES".to_string(),
                    reason: "must be between 1 and 1000000".to_string(),
                });
            }
        }
        if let Some(value) = nonempty_var("CACHE_DIR") {
            s.cache_dir = value;
        }
        if let Some(value) = nonempty_var("CACHE_PERSIST") {
            s.cache_persist = parse_bool("CACHE_PERSIST", &value)?;
        }

        for (name, value) in [
            ("RETRY_BACKOFF_SECONDS", s.retry_backoff_seconds),
            (
                "UPSTREAM_MIN_INTERVAL_SECONDS",
                s.upstream_min_interval_seconds,
            ),
            ("UPSTREAM_JITTER_SECONDS", s.upstream_jitter_seconds),
            (
                "CIRCUIT_BREAKER_COOLDOWN_SECONDS",
                s.circuit_breaker_cooldown_seconds,
            ),
        ] {
            if !(value.is_finite() && value >= 0.0) {
                return Err(ConfigError::Invalid {
                    name: name.to_string(),
                    reason: "must be a finite, nonnegative number".to_string(),
                });
            }
        }

        if s.environment == Environment::Production && s.cors_origins.iter().any(|o| o == "*") {
            tracing::warn!(
                event = "config.cors_wildcard",
                "CORS is set to '*' in production"
            );
        }
        s.cookie_fingerprint = s.fingerprint();
        Ok(s)
    }

    pub fn li_at(&self) -> Option<&str> {
        self.linkedin_li_at.as_deref()
    }

    pub fn jsessionid(&self) -> Option<&str> {
        self.linkedin_jsessionid.as_deref()
    }

    pub fn extra_cookies(&self) -> &[(String, String)] {
        &self.linkedin_extra_cookies
    }

    pub fn email(&self) -> Option<&str> {
        self.linkedin_email.as_deref()
    }

    pub fn password(&self) -> Option<&str> {
        self.linkedin_password.as_deref()
    }

    pub fn cookie_header(&self) -> Option<&str> {
        self.linkedin_cookie_header.as_deref()
    }

    pub fn profile_query_id(&self) -> Option<&str> {
        self.linkedin_profile_query_id.as_deref()
    }

    pub fn proxy_url(&self) -> Option<&str> {
        self.proxy_url.as_deref()
    }

    pub fn auth_required(&self) -> bool {
        self.billing_enabled || !self.api_keys.is_empty()
    }

    pub fn has_cookie_credentials(&self) -> bool {
        self.linkedin_li_at.is_some() || self.linkedin_cookie_header.is_some()
    }

    pub fn has_password_credentials(&self) -> bool {
        self.allow_password_login
            && self.linkedin_email.is_some()
            && self.linkedin_password.is_some()
    }

    pub fn is_production(&self) -> bool {
        self.environment == Environment::Production
    }
}

fn parse_i64(name: &str, value: &str) -> Result<i64, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        name: name.to_string(),
        reason: format!("'{value}' is not a number"),
    })
}

fn parse_u64(name: &str, value: &str) -> Result<u64, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        name: name.to_string(),
        reason: format!("'{value}' is not a number"),
    })
}

fn parse_usize(name: &str, value: &str) -> Result<usize, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        name: name.to_string(),
        reason: format!("'{value}' is not a number"),
    })
}

fn parse_f64(name: &str, value: &str) -> Result<f64, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        name: name.to_string(),
        reason: format!("'{value}' is not a number"),
    })
}

fn parse_bool(name: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::Invalid {
            name: name.to_string(),
            reason: format!("'{value}' is not a boolean"),
        }),
    }
}

fn nonempty_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn blank_to_none(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Cookie values carried across env-var transports (Coolify UI escapes
/// `"` as `\"`; some pastes keep raw `"`). Neither form is what LinkedIn
/// wants — its cookie parser accepts the bare value — so collapse both:
/// `\"v=2&...\"` / `"v=2&..."` -> `v=2&...`.
fn unescape_quotes(value: &str) -> String {
    value.replace("\\\"", "\"").replace('"', "")
}

/// Heads of the security-relevant cookies plus their total length, so the
/// startup log can prove which cookie set the container received and
/// whether it was truncated, without printing a secret. Covers both the
/// combined header and the split env vars.
fn fingerprint_cookie_header(header: &str) -> String {
    fn head(value: &str, n: usize) -> String {
        let v = value.trim();
        v.chars().take(n).collect()
    }
    let mut li_at = String::new();
    let mut jsessionid = String::new();
    let mut cf_bm = String::new();
    for piece in header.split(';') {
        let Some((name, value)) = piece.split_once('=') else {
            continue;
        };
        match name.trim() {
            "li_at" => li_at = head(value, 12),
            "JSESSIONID" => jsessionid = head(value, 12),
            "__cf_bm" => cf_bm = head(value, 12),
            _ => {}
        }
    }
    format!(
        "len={} li_at={} JSESSIONID={} cf_bm={}",
        header.len(),
        if li_at.is_empty() { "MISSING" } else { li_at.as_str() },
        if jsessionid.is_empty() { "MISSING" } else { jsessionid.as_str() },
        if cf_bm.is_empty() { "MISSING" } else { cf_bm.as_str() },
    )
}

impl Settings {
    /// Fingerprint over whichever cookie source is configured — the full
    /// header, or the split individual env vars.
    fn fingerprint(&self) -> Option<String> {
        if let Some(header) = self.linkedin_cookie_header.as_deref() {
            return Some(fingerprint_cookie_header(header));
        }
        let mut li_at = String::new();
        let mut jsessionid = String::new();
        if let Some(v) = self.linkedin_li_at.as_deref() {
            li_at = v.chars().take(12).collect();
        }
        if let Some(v) = self.linkedin_jsessionid.as_deref() {
            jsessionid = v.chars().take(12).collect();
        }
        if li_at.is_empty() {
            return None;
        }
        Some(format!(
            "split li_at={} JSESSIONID={} extra_cookies={:?}",
            li_at,
            jsessionid,
            self.linkedin_extra_cookies.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ))
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                None
            } else {
                Some(item.to_string())
            }
        })
        .collect()
}

fn parse_csv(name: &str) -> Vec<String> {
    match nonempty_var(name) {
        Some(value) => split_csv(&value),
        None => Vec::new(),
    }
}

fn validate_proxy_url(value: &str) -> Result<String, ConfigError> {
    let parsed = Url::parse(value).map_err(|_| ConfigError::Invalid {
        name: "PROXY_URL".to_string(),
        reason: format!("'{value}' is not a parseable URL"),
    })?;
    if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err(ConfigError::Invalid {
            name: "PROXY_URL".to_string(),
            reason: format!("unsupported scheme '{}'", parsed.scheme()),
        });
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parsing() {
        assert_eq!(split_csv(" a, b ,,c"), vec!["a", "b", "c"]);
        assert_eq!(split_csv(""), Vec::<String>::new());
        assert_eq!(split_csv(" , "), Vec::<String>::new());
    }

    #[test]
    fn boolean_parsing() {
        assert!(parse_bool("X", "TRUE").unwrap());
        assert!(!parse_bool("X", "0").unwrap());
        assert!(parse_bool("X", "yes").unwrap());
        assert!(parse_bool("X", "nonsense").is_err());
    }

    #[test]
    fn number_parsing() {
        assert_eq!(parse_u64("X", "42").unwrap(), 42);
        assert!(parse_u64("X", "4.2").is_err());
        assert!(parse_u64("X", "-1").is_err());
        assert_eq!(parse_f64("X", "1.5").unwrap(), 1.5);
        // f64::from_str accepts "inf"/"nan"; the validation loop rejects
        // non-finite values at startup (see Settings::from_env).
        assert!(parse_f64("X", "1e999").unwrap().is_infinite());
        assert!(parse_f64("X", "inf").unwrap().is_infinite());
        assert!(parse_f64("X", "nan").unwrap().is_nan());
    }

    #[test]
    fn environment_parsing() {
        assert_eq!(
            Environment::parse("PRODUCTION").unwrap(),
            Environment::Production
        );
        assert!(Environment::parse("prod").is_err());
    }

    #[test]
    fn proxy_url_validation() {
        assert!(validate_proxy_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_proxy_url("https://proxy.example:443").is_ok());
        assert!(validate_proxy_url("socks5://127.0.0.1:1080").is_ok());
        assert!(validate_proxy_url("ftp://x.y").is_err());
        assert!(validate_proxy_url("not a url").is_err());
    }

    #[test]
    fn quote_escapes_are_collapsed() {
        assert_eq!(unescape_quotes("v=2&abc"), "v=2&abc");
        assert_eq!(unescape_quotes("\"v=2&abc\""), "v=2&abc");
        assert_eq!(unescape_quotes("\\\"v=2&abc\\\""), "v=2&abc");
        assert_eq!(unescape_quotes(r#"\"v=2&abc\""#), "v=2&abc");
    }

    #[test]
    fn secrets_redacted_in_debug() {
        let s = Settings {
            api_keys: vec!["super-secret-key".to_string()],
            linkedin_li_at: Some("li_at:secret".to_string()),
            linkedin_password: Some("hunter2".to_string()),
            linkedin_cookie_header: Some("li_at=abc123; JSESSIONID=\"xyz\"".to_string()),
            ..Settings::default()
        };
        let rendered = format!("{s:?}");
        assert!(!rendered.contains("super-secret-key"));
        assert!(!rendered.contains("li_at:secret"));
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("abc123"));
    }

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.port, 8000);
        assert_eq!(s.cache_max_entries, 512);
        assert!(!s.auth_required());
        assert!(s.max_retries <= 10);
        assert!(s.upstream_max_concurrency >= 1);
    }
}
