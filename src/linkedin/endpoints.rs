//! Central definition of every LinkedIn URL and request shape this service
//! uses. The client sends these; it never builds paths itself.

use std::borrow::Cow;

pub const BASE_URL: &str = "https://www.linkedin.com";
pub const VOYAGER_PREFIX: &str = "/voyager/api";
pub const AUTH_SEED_URL: &str = "/uas/authenticate";
pub const AUTH_SUBMIT_URL: &str = "/uas/authenticate";
pub const ME: &str = "/voyager/api/me";

pub const DASH_DECORATION: &str = "com.linkedin.voyager.dash.deco.identity.profile";
pub const DASH_DECORATION_VERSION: usize = 101;

#[derive(Debug, Clone)]
pub struct VoyagerCall {
    pub name: &'static str,
    pub path: String,
    pub params: Vec<(Cow<'static, str>, String)>,
    pub required: bool,
    /// Request the normalised envelope (`data` + `included`) for Dash calls.
    pub normalized: bool,
}

impl VoyagerCall {
    pub fn describe(&self) -> &'static str {
        self.name
    }
}

fn seg(value: &str) -> String {
    // Percent-encode every byte except unreserved characters — equivalent to
    // Python's urllib.parse.quote(value, safe="").
    const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut out = String::with_capacity(value.len() * 3);
    for &byte in value.as_bytes() {
        if UNRESERVED.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn fsd_urn(profile_id: &str) -> String {
    if profile_id.starts_with("urn:") {
        profile_id.to_string()
    } else {
        format!("urn:li:fsd_profile:{profile_id}")
    }
}

/// Legacy full profile projection — superseded by Dash; kept as a fallback.
pub fn profile_view(identifier: &str) -> VoyagerCall {
    VoyagerCall {
        name: "profileView",
        path: format!(
            "{VOYAGER_PREFIX}/identity/profiles/{}/profileView",
            seg(identifier)
        ),
        params: Vec::new(),
        required: false,
        normalized: false,
    }
}

/// Core member record: names, headline, geo, profile and cover images.
pub fn profile_core(identifier: &str) -> VoyagerCall {
    VoyagerCall {
        name: "profile",
        path: format!("{VOYAGER_PREFIX}/identity/profiles/{}", seg(identifier)),
        params: Vec::new(),
        required: false,
        normalized: false,
    }
}

/// Contact card: websites, email, phone, birthday, Twitter handles.
pub fn profile_contact_info(identifier: &str) -> VoyagerCall {
    VoyagerCall {
        name: "contactInfo",
        path: format!(
            "{VOYAGER_PREFIX}/identity/profiles/{}/profileContactInfo",
            seg(identifier)
        ),
        params: Vec::new(),
        required: false,
        normalized: false,
    }
}

/// Paginated skills, which profileView truncates.
pub fn profile_skills(identifier: &str, count: usize) -> VoyagerCall {
    VoyagerCall {
        name: "skills",
        path: format!(
            "{VOYAGER_PREFIX}/identity/profiles/{}/skills",
            seg(identifier)
        ),
        params: vec![
            (Cow::Borrowed("count"), count.to_string()),
            (Cow::Borrowed("start"), "0".to_string()),
        ],
        required: false,
        normalized: false,
    }
}

/// Follower and connection counts plus the viewer's distance to the member.
pub fn profile_network_info(identifier: &str) -> VoyagerCall {
    VoyagerCall {
        name: "networkInfo",
        path: format!(
            "{VOYAGER_PREFIX}/identity/profiles/{}/networkinfo",
            seg(identifier)
        ),
        params: Vec::new(),
        required: false,
        normalized: false,
    }
}

/// The current full-profile call: the member plus every section entity in one
/// decorated collection.
pub fn dash_profile(identifier: &str) -> VoyagerCall {
    VoyagerCall {
        name: "dashProfile",
        path: format!("{VOYAGER_PREFIX}/identity/dash/profiles"),
        params: vec![
            (Cow::Borrowed("q"), "memberIdentity".to_string()),
            (Cow::Borrowed("memberIdentity"), identifier.to_string()),
            (
                Cow::Borrowed("decorationId"),
                format!("{DASH_DECORATION}.FullProfileWithEntities-{DASH_DECORATION_VERSION}"),
            ),
        ],
        required: true,
        normalized: true,
    }
}

/// The same collection without a decoration id: fewer entities, fewer ways to
/// fail.
pub fn dash_profile_minimal(identifier: &str) -> VoyagerCall {
    VoyagerCall {
        name: "dashProfileMinimal",
        path: format!("{VOYAGER_PREFIX}/identity/dash/profiles"),
        params: vec![
            (Cow::Borrowed("q"), "memberIdentity".to_string()),
            (Cow::Borrowed("memberIdentity"), identifier.to_string()),
        ],
        required: false,
        normalized: true,
    }
}

/// GraphQL variant, used only when a query id is configured. Hashes rotate per
/// web client release, so the query id is configuration, never code.
/// `memberIdentity` accepts both a vanity slug and a bare URN id (verified
/// live 2026-08-30); `vanityName` is rejected by current query ids.
pub fn graphql_profile(identifier: &str, query_id: &str) -> VoyagerCall {
    VoyagerCall {
        name: "graphqlProfile",
        path: format!("{VOYAGER_PREFIX}/graphql"),
        params: vec![
            (Cow::Borrowed("includeWebMetadata"), "true".to_string()),
            (
                Cow::Borrowed("variables"),
                format!("(memberIdentity:{})", seg(identifier)),
            ),
            (Cow::Borrowed("queryId"), query_id.to_string()),
        ],
        required: false,
        normalized: true,
    }
}

/// Contact card in the Dash model, keyed by the profile id.
pub fn dash_contact_info(profile_id: &str) -> VoyagerCall {
    VoyagerCall {
        name: "dashContactInfo",
        path: format!(
            "{VOYAGER_PREFIX}/identity/dash/profiles/{}",
            seg(&fsd_urn(profile_id))
        ),
        params: vec![(
            Cow::Borrowed("decorationId"),
            format!("{DASH_DECORATION}.WebProfileContactInfo-4"),
        )],
        required: false,
        normalized: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_build_expected_paths() {
        assert_eq!(
            profile_view("Ada Lovelace").path,
            "/voyager/api/identity/profiles/Ada%20Lovelace/profileView"
        );
        assert_eq!(
            dash_profile("adalovelace").params[2].1,
            format!("{DASH_DECORATION}.FullProfileWithEntities-{DASH_DECORATION_VERSION}")
        );
        let skills = profile_skills("adalovelace", 100);
        assert!(
            skills
                .params
                .contains(&(Cow::Borrowed("count"), "100".to_string()))
        );
        assert!(
            skills
                .params
                .contains(&(Cow::Borrowed("start"), "0".to_string()))
        );
        let dash_contact = dash_contact_info("urn:li:fsd_profile:ABC");
        assert_eq!(
            dash_contact.path,
            "/voyager/api/identity/dash/profiles/urn%3Ali%3Afsd_profile%3AABC"
        );
        let dash_contact2 = dash_contact_info("ACoA-x");
        assert_eq!(
            dash_contact2.path,
            "/voyager/api/identity/dash/profiles/urn%3Ali%3Afsd_profile%3AACoA-x"
        );
        let gql = graphql_profile("adalovelace", "abc:def");
        assert!(gql.normalized);
        assert_eq!(gql.params[1].1, "(memberIdentity:adalovelace)");
    }

    #[test]
    fn identifier_encoding_is_path_segment_safe() {
        let call = profile_view("über/dev");
        assert_eq!(
            call.path,
            "/voyager/api/identity/profiles/%C3%BCber%2Fdev/profileView"
        );
    }
}
