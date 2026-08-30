//! Golden fixture parity: the assembler output for both Dash envelope forms and
//! the legacy fallback must serialize byte-for-byte to `expected_profile.json`.

mod support;

use chrono::{DateTime, Utc};
use serde_json::Value;

use support::fixture;
use tross::domain::profile::Profile;
use tross::parser::assembler::{build_profile, draft_from_dash, draft_from_legacy};
use tross::parser::draft::ProfileDraft;

const NOW: (i64, i64) = (2026, 8);

fn fetched_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .expect("canned timestamp")
        .with_timezone(&Utc)
}

fn expected() -> Value {
    fixture("expected_profile.json")
}

fn build(draft: ProfileDraft) -> Profile {
    build_profile(
        "adalovelace",
        "vanity",
        "https://www.linkedin.com/in/adalovelace/",
        &draft,
        vec![
            tross::domain::profile::SourceCall {
                endpoint: "dashProfile".into(),
                status_code: 200,
                ok: true,
                elapsed_ms: 12,
                attempts: 1,
            },
            tross::domain::profile::SourceCall {
                endpoint: "contactInfo".into(),
                status_code: 200,
                ok: true,
                elapsed_ms: 8,
                attempts: 1,
            },
            tross::domain::profile::SourceCall {
                endpoint: "skills".into(),
                status_code: 200,
                ok: true,
                elapsed_ms: 6,
                attempts: 1,
            },
            tross::domain::profile::SourceCall {
                endpoint: "networkInfo".into(),
                status_code: 200,
                ok: true,
                elapsed_ms: 5,
                attempts: 1,
            },
            tross::domain::profile::SourceCall {
                endpoint: "dashContactInfo".into(),
                status_code: 200,
                ok: true,
                elapsed_ms: 4,
                attempts: 1,
            },
        ],
        Vec::new(),
        fetched_at(),
    )
}

fn assert_golden(actual: &Profile) {
    let actual_json = serde_json::to_value(actual).expect("serialize profile");
    let expected_json = expected();
    assert_eq!(
        actual_json, expected_json,
        "parser output diverges from golden fixture; diff the JSON to see which field"
    );
}

#[test]
fn golden_embedded_dash_matches() {
    let payload = fixture("dash_embedded.json");
    let draft = draft_from_dash(Some(&payload), "dashProfile", NOW).expect("draft");
    assert_golden(&build(draft));
}

#[test]
fn golden_normalized_dash_matches() {
    let payload = fixture("dash_normalized.json");
    let draft = draft_from_dash(Some(&payload), "dashProfile", NOW).expect("draft");
    assert_golden(&build(draft));
}

/// The legacy projection carries no pictures/network/contact enrichment, so it
/// cannot equal the dash golden byte-for-byte; it must parse without panicking
/// and populate the sections its fixture carries.
#[test]
fn legacy_parses_all_sections() {
    let payload = fixture("legacy_profile_view.json");
    let draft = draft_from_legacy(Some(&payload), None, NOW).expect("draft");
    let profile = build(draft);
    assert_eq!(profile.full_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(
        profile.meta.sections_populated.len(),
        13,
        "every section populated"
    );
    assert_eq!(profile.experience.len(), 2);
    assert_eq!(profile.education.len(), 1);
    assert_eq!(profile.certifications.len(), 1);
}

#[test]
fn both_dash_forms_are_equivalent() {
    let embedded =
        draft_from_dash(Some(&fixture("dash_embedded.json")), "dashProfile", NOW).unwrap();
    let normalized =
        draft_from_dash(Some(&fixture("dash_normalized.json")), "dashProfile", NOW).unwrap();
    let e = serde_json::to_value(build(embedded)).unwrap();
    let n = serde_json::to_value(build(normalized)).unwrap();
    assert_eq!(e, n, "the two Dash envelopes must produce identical output");
}
