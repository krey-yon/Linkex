//! Parsing and canonicalisation of LinkedIn profile URLs.
//!
//! The public API accepts anything a human might paste: a full desktop URL, a
//! country subdomain, a mobile link, a URL with tracking parameters, or the
//! bare vanity name. Everything is reduced to the identifier the Voyager API
//! expects. Host validation uses `url::Url` host parsing, not string splits.

use std::fmt;

use url::Url;

use crate::error::AppError;

pub const MAX_IDENTIFIER_LEN: usize = 120;

const ALLOWED_HOST: &str = "linkedin.com";
const LOCALE_SUFFIXES: &[&str] = &[
    "en", "de", "fr", "es", "it", "pt", "nl", "ru", "ja", "zh", "ko", "ar", "hi",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierKind {
    Vanity,
    Obfuscated,
}

impl IdentifierKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            IdentifierKind::Vanity => "vanity",
            IdentifierKind::Obfuscated => "obfuscated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProfileRef {
    pub identifier: String,
    pub kind: IdentifierKind,
    pub canonical_url: String,
    pub raw_input: String,
}

impl ProfileRef {
    pub fn cache_key(&self) -> String {
        format!("profile:{}", self.identifier.to_lowercase())
    }
}

impl fmt::Display for ProfileRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProfileRef({}, {})", self.identifier, self.kind.as_str())
    }
}

fn invalid_profile_url(message: &str) -> AppError {
    AppError::InvalidProfileUrl {
        message: message.to_string(),
        details: serde_json::Map::new(),
    }
}

pub fn parse_profile_url(value: &str) -> Result<ProfileRef, AppError> {
    if value.trim().is_empty() {
        return Err(invalid_profile_url("A LinkedIn profile URL is required."));
    }
    let raw = value.trim();
    let mut candidate = raw.to_string();
    if candidate.to_lowercase().contains("linkedin.com") && !candidate.contains("//") {
        candidate = format!("https://{candidate}");
    }

    let identifier = if candidate.contains("//") {
        identifier_from_url(&candidate)?
    } else {
        clean_segment(&candidate)?
    };

    let kind = if is_obfuscated(&identifier) {
        IdentifierKind::Obfuscated
    } else {
        IdentifierKind::Vanity
    };
    Ok(ProfileRef {
        canonical_url: format!("https://www.linkedin.com/in/{identifier}/"),
        identifier,
        kind,
        raw_input: raw.to_string(),
    })
}

fn identifier_from_url(candidate: &str) -> Result<String, AppError> {
    let parsed = Url::parse(candidate)
        .map_err(|_| invalid_profile_url("The supplied URL could not be parsed."))?;

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(invalid_profile_url(
            "Only linkedin.com profile URLs are supported.",
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| invalid_profile_url("Only linkedin.com profile URLs are supported."))?;
    if !is_allowed_host(host) {
        return Err(invalid_profile_url(
            "Only linkedin.com profile URLs are supported.",
        ));
    }

    let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(invalid_profile_url("The URL does not point at a profile."));
    }

    let lowered: Vec<String> = segments.iter().map(|s| s.to_lowercase()).collect();
    if let Some(index) = lowered.iter().position(|s| s == "in") {
        let remainder = &segments[index + 1..];
        if remainder.is_empty() {
            return Err(invalid_profile_url(
                "The URL is missing the profile identifier.",
            ));
        }
        return clean_segment(remainder[0]);
    }

    match lowered[0].as_str() {
        "pub" | "profile" => Err(invalid_profile_url(
            "Legacy /pub and /profile links are not supported. Use the /in/ URL.",
        )),
        "company" | "school" | "showcase" | "groups" | "jobs" | "posts" | "feed" => {
            Err(invalid_profile_url(&format!(
                "That is a LinkedIn {} URL, not a member profile.",
                lowered[0]
            )))
        }
        _ => Err(invalid_profile_url(
            "The URL does not contain an /in/ profile path.",
        )),
    }
}

fn is_allowed_host(host: &str) -> bool {
    let host = host.to_lowercase();
    host == ALLOWED_HOST || host.ends_with(".linkedin.com")
}

fn is_obfuscated(identifier: &str) -> bool {
    identifier.starts_with("ACoA")
        && identifier[4..].len() >= 10
        && identifier[4..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn clean_segment(segment: &str) -> Result<String, AppError> {
    let identifier = percent_decode(segment);
    let mut identifier = identifier.trim().trim_matches('/').to_string();
    if let Some(q) = identifier.find('?') {
        identifier.truncate(q);
    }
    if let Some(h) = identifier.find('#') {
        identifier.truncate(h);
    }

    if LOCALE_SUFFIXES.contains(&identifier.to_lowercase().as_str()) {
        return Err(invalid_profile_url(
            "The URL is missing the profile identifier.",
        ));
    }
    if identifier.is_empty() || !is_valid_vanity(segment, &identifier) {
        return Err(invalid_profile_url(
            "The profile identifier contains unsupported characters.",
        ));
    }
    Ok(identifier)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(code) = u8::from_str_radix(&value[i + 1..i + 3], 16)
        {
            out.push(code);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Validate against the Python `_VANITY` regex: `[A-Za-z0-9\u00C0-\u024F\u0400-\u04FF%._-]{2,120}`.
fn is_valid_vanity(raw_segment: &str, decoded: &str) -> bool {
    let ok = |s: &str| {
        (2..=MAX_IDENTIFIER_LEN).contains(&s.chars().count())
            && s.chars().all(|c| {
                matches!(c,
                    'A'..='Z' | 'a'..='z' | '0'..='9' | '%' | '.' | '_' | '-'
                    | '\u{00C0}'..='\u{024F}' | '\u{0400}'..='\u{04FF}')
            })
    };
    ok(raw_segment) || ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(value: &str) -> ProfileRef {
        parse_profile_url(value).expect("expected a valid profile URL")
    }

    fn err(value: &str) -> AppError {
        parse_profile_url(value).expect_err("expected an invalid profile URL")
    }

    #[test]
    fn accepts_canonical_and_tracked_urls() {
        for url in [
            "https://www.linkedin.com/in/williamhgates/",
            "https://www.linkedin.com/in/williamhgates",
            "https://www.linkedin.com/in/williamhgates?originalSubdomain=us&trk=feed",
            "https://www.linkedin.com/in/williamhgates/#some-fragment",
            "http://www.linkedin.com/in/williamhgates/",
            "www.linkedin.com/in/williamhgates/",
            "linkedin.com/in/williamhgates",
            "https://de.linkedin.com/in/williamhgates/",
            "https://in.linkedin.com/in/williamhgates/",
        ] {
            assert_eq!(ok(url).identifier, "williamhgates", "case: {url}");
        }
    }

    #[test]
    fn accepts_bare_vanity_and_mobile_forms() {
        assert_eq!(ok("williamhgates").identifier, "williamhgates");
        assert_eq!(ok("WilliamGates").identifier, "WilliamGates");
        assert_eq!(
            ok("https://www.linkedin.com/mwlite/in/williamhgates").identifier,
            "williamhgates"
        );
    }

    #[test]
    fn accepts_obfuscated_and_percent_encoded_identifiers() {
        let obf = ok("ACoAAB1c9P0BbXYZ123456789");
        assert_eq!(obf.kind, IdentifierKind::Obfuscated);
        // Python unquotes then validates the raw segment, so a decoded space
        // is kept in the identifier.
        let decoded = ok("https://www.linkedin.com/in/alias%20gates/");
        assert_eq!(decoded.identifier, "alias gates");
        let unicode = ok("https://www.linkedin.com/in/übermensch/");
        assert_eq!(unicode.identifier, "übermensch");
    }

    #[test]
    fn canonical_url_and_cache_key() {
        let ref_ = ok("https://www.linkedin.com/in/WilliamGates/?x=1");
        assert_eq!(
            ref_.canonical_url,
            "https://www.linkedin.com/in/WilliamGates/"
        );
        assert_eq!(ref_.cache_key(), "profile:williamgates");
    }

    #[test]
    fn rejects_empty_and_blank() {
        assert!(err("").to_string().contains("required"));
        assert!(err("   ").to_string().contains("required"));
    }

    #[test]
    fn rejects_hostile_hosts() {
        for host in [
            "https://linkedin.com.attacker.test/in/x",
            "https://linkedin.com.evil.com/in/x",
            "https://attacker.test/linkedin.com/in/x",
            "https://evil.com/in/x",
            "https://linkedin.com@attacker.test/in/x",
            "https://user:pass@linkedin.com/in/x",
            "https://127.0.0.1/in/x",
            "https://a.in/in/x",
        ] {
            let result = err(host);
            assert!(
                result.to_string().contains("linkedin.com"),
                "opps host case: {host}\n{}",
                result
            );
        }
    }

    #[test]
    fn rejects_non_profile_paths() {
        for path in [
            "https://www.linkedin.com/company/acme/",
            "https://www.linkedin.com/school/mit/",
            "https://www.linkedin.com/showcase/foo/",
            "https://www.linkedin.com/groups/123/",
            "https://www.linkedin.com/jobs/view/123/",
            "https://www.linkedin.com/posts/foo",
            "https://www.linkedin.com/feed/",
            "https://www.linkedin.com/pub/foo",
            "https://www.linkedin.com/profile/foo",
            "https://www.linkedin.com/",
            "https://www.linkedin.com/not/a/profile",
        ] {
            let result = err(path);
            assert!(
                matches!(result, AppError::InvalidProfileUrl { .. }),
                "case: {path}"
            );
        }
    }

    #[test]
    fn rejects_missing_or_locale_only_identifiers() {
        assert!(
            err("https://www.linkedin.com/in/")
                .to_string()
                .contains("missing")
        );
        assert!(
            err("https://www.linkedin.com/in/en/")
                .to_string()
                .contains("missing")
        );
    }

    #[test]
    fn rejects_unsupported_characters() {
        assert!(
            err("https://www.linkedin.com/in/some!ch@ra/")
                .to_string()
                .contains("unsupported")
        );
        assert!(err("x").to_string().contains("unsupported"));
        assert!(err("ab>").to_string().contains("unsupported"));
    }

    #[test]
    fn rejects_excessively_long_identifiers() {
        let long = "a".repeat(121);
        assert!(err(&long).to_string().contains("unsupported"));
    }

    #[test]
    fn obfuscated_prefix_is_exact_cased() {
        // python: ^ACoA — a lowercase prefix is a plain vanity name
        let ref_ = ok("acoAB1c9P0BbXYZ123456789");
        assert_eq!(ref_.kind, IdentifierKind::Vanity);
        // Short ACoA prefix is still a valid vanity name
        assert_eq!(ok("ACoA123").identifier, "ACoA123");
    }
}
