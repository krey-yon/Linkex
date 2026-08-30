//! Assembler: the seam between "what LinkedIn sent" and "what this API
//! publishes". Pure — the same draft always produces the same document,
//! which is what makes fixture comparisons meaningful without network access.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};

use serde_json::{Map, Value};

use crate::domain::profile::{Profile, ProfileMeta, SourceCall};

use super::common::elements;
use super::contact::parse_contact_info;
use super::dash::parse_dash_profile;
use super::draft::ProfileDraft;
use super::draft::SECTION_FIELDS;
use super::legacy;
use super::legacy::parse_skill_elements;

static EMPTY_VIEW: LazyLock<Map<String, Value>> = LazyLock::new(Map::new);

pub fn draft_from_legacy(
    profile_view: Option<&Value>,
    core: Option<&Value>,
    now: (i64, i64),
) -> Option<ProfileDraft> {
    let profile_view = profile_view.and_then(Value::as_object);
    let core = core.and_then(Value::as_object);
    let has_content = profile_view.is_some_and(|v| v.get("profile").is_some()) || core.is_some();
    if !has_content {
        return None;
    }
    let view = profile_view.unwrap_or(&EMPTY_VIEW);
    let core = core.unwrap_or(&EMPTY_VIEW);

    let draft = ProfileDraft {
        identity: legacy::parse_identity([
            view.get("profile").and_then(Value::as_object),
            Some(core),
        ]),
        sections: crate::parser::draft::Sections {
            experience: legacy::parse_experience(view, now),
            education: legacy::parse_education(view, now),
            skills: legacy::parse_skills(view),
            certifications: legacy::parse_certifications(view, now),
            languages: legacy::parse_languages(view),
            projects: legacy::parse_projects(view, now),
            publications: legacy::parse_publications(view),
            honors: legacy::parse_honors(view),
            volunteering: legacy::parse_volunteering(view, now),
            courses: legacy::parse_courses(view),
            patents: legacy::parse_patents(view),
            test_scores: legacy::parse_test_scores(view),
            organizations: legacy::parse_organizations(view, now),
        },
        network: None,
        contact: None,
        strategy: "profileView".to_string(),
    };
    if draft.is_populated() {
        Some(draft)
    } else {
        None
    }
}

pub fn draft_from_dash(
    payload: Option<&Value>,
    strategy: &str,
    now: (i64, i64),
) -> Option<ProfileDraft> {
    let parsed = parse_dash_profile(payload?, now)?;
    let draft = ProfileDraft {
        identity: parsed.identity,
        sections: parsed.sections,
        network: parsed.network,
        contact: parsed.contact,
        strategy: strategy.to_string(),
    };
    if draft.is_populated() {
        Some(draft)
    } else {
        None
    }
}

pub fn draft_from_payloads(payloads: &Map<String, Value>, now: (i64, i64)) -> Option<ProfileDraft> {
    for name in ["dashProfile", "dashProfileMinimal", "graphqlProfile"] {
        if let Some(draft) = draft_from_dash(payloads.get(name), name, now) {
            return Some(draft);
        }
    }
    draft_from_legacy(payloads.get("profileView"), payloads.get("profile"), now)
}

/// Fold optional enrichment payloads into *draft*.
pub fn enrich(draft: &mut ProfileDraft, payloads: &Map<String, Value>) {
    let dedicated: Vec<&Map<String, Value>> = payloads
        .get("skills")
        .and_then(Value::as_object)
        .map(|p| elements(p, "elements"))
        .unwrap_or_default();
    draft.adopt_skills(parse_skill_elements(dedicated));

    let network = payloads
        .get("networkInfo")
        .and_then(Value::as_object)
        .and_then(legacy::parse_network);
    draft.fill_network(network);

    let contact = payloads
        .get("contactInfo")
        .or_else(|| payloads.get("dashContactInfo"))
        .and_then(parse_contact_info);
    draft.merge_contact(contact);
}

pub fn build_profile(
    ref_identifier: &str,
    ref_kind: &str,
    ref_canonical_url: &str,
    draft: &ProfileDraft,
    sources: Vec<SourceCall>,
    warnings: Vec<String>,
    now: DateTime<Utc>,
) -> Profile {
    let identity = &draft.identity;
    let public_identifier = identity.public_identifier.clone().or_else(|| {
        if ref_kind == "vanity" {
            Some(ref_identifier.to_string())
        } else {
            None
        }
    });

    let populated = draft.populated_sections();
    let completeness = if populated.is_empty() {
        0.0
    } else {
        (populated.len() as f64 / SECTION_FIELDS.len() as f64 * 1000.0).round() / 1000.0
    };

    Profile {
        profile_url: match &public_identifier {
            Some(id) => format!("https://www.linkedin.com/in/{id}/"),
            None => ref_canonical_url.to_string(),
        },
        public_identifier,
        member_urn: identity.member_urn.clone(),
        profile_id: identity.profile_id.clone(),
        first_name: identity.first_name.clone(),
        last_name: identity.last_name.clone(),
        full_name: identity.full_name.clone(),
        headline: identity.headline.clone(),
        about: identity.about.clone(),
        industry: identity.industry.clone(),
        pronouns: identity.pronouns.clone(),
        location: identity.location.clone(),
        profile_picture: identity.profile_picture.clone(),
        background_picture: identity.background_picture.clone(),
        network: draft.network.clone(),
        contact: draft.contact.clone(),
        experience: draft.sections.experience.clone(),
        education: draft.sections.education.clone(),
        skills: draft.sections.skills.clone(),
        certifications: draft.sections.certifications.clone(),
        languages: draft.sections.languages.clone(),
        projects: draft.sections.projects.clone(),
        publications: draft.sections.publications.clone(),
        honors: draft.sections.honors.clone(),
        volunteering: draft.sections.volunteering.clone(),
        courses: draft.sections.courses.clone(),
        patents: draft.sections.patents.clone(),
        test_scores: draft.sections.test_scores.clone(),
        organizations: draft.sections.organizations.clone(),
        meta: ProfileMeta {
            fetched_at: now,
            sources,
            warnings,
            sections_populated: populated.into_iter().map(str::to_string).collect(),
            completeness,
        },
    }
}

#[cfg(test)]
mod malformed_payloads {
    //! Upstream data is hostile: wrong types, nulls, empty envelopes. None of
    //! it may panic, and garbage must produce no draft at all.

    use chrono::Utc;
    use serde_json::{Map, Value, json};

    use super::*;

    fn map(payload: Value) -> Map<String, Value> {
        payload
            .as_object()
            .expect("test payload is an object")
            .clone()
    }

    #[test]
    fn garbage_payloads_yield_no_draft() {
        for payload in [
            json!({}),
            json!({"dashProfile": 42}),
            json!({"dashProfile": null}),
            json!({"dashProfile": [1, 2]}),
            json!({"dashProfile": {"included": [], "data": {}}}),
            json!({"profileView": "nope"}),
            json!({"profileView": {"profile": 42}}),
            json!({"profileView": {"profile": {"firstName": 7}}}),
        ] {
            let payloads = map(payload);
            assert!(
                draft_from_payloads(&payloads, (2026, 8)).is_none(),
                "garbage payload produced a draft: {payloads:?}"
            );
        }
    }

    #[test]
    fn wrong_typed_collections_degrade_to_empty() {
        let payload = json!({
            "firstName": "Ada",
            "publicIdentifier": "ada",
            "profilePositions": 42,
            "profileEducations": {"elements": "nope"},
            "profileSkills": [1, 2],
        });
        let draft = draft_from_dash(Some(&payload), "dashProfile", (2026, 8))
            .expect("identity is present, sections are garbage");
        assert_eq!(draft.identity.public_identifier.as_deref(), Some("ada"));
        assert_eq!(draft.identity.full_name.as_deref(), Some("Ada"));
        assert!(draft.sections.experience.is_empty());
        assert!(draft.sections.education.is_empty());
        assert!(draft.sections.skills.is_empty());
    }

    #[test]
    fn enrich_with_wrong_types_is_noop() {
        let payload = json!({"firstName": "Ada", "publicIdentifier": "ada"});
        let mut draft = draft_from_dash(Some(&payload), "dashProfile", (2026, 8)).expect("draft");
        enrich(
            &mut draft,
            &map(json!({
                "skills": 42,
                "networkInfo": [],
                "contactInfo": "nope",
                "dashContactInfo": {"emailAddress": 7},
            })),
        );
        assert!(draft.sections.skills.is_empty());
        assert!(draft.network.is_none());
        assert!(draft.contact.is_none());
    }

    #[test]
    fn sparse_draft_builds_all_null_profile() {
        let payload = json!({"publicIdentifier": "sparse"});
        let draft = draft_from_dash(Some(&payload), "dashProfile", (2026, 8)).expect("draft");
        let profile = build_profile(
            "sparse",
            "vanity",
            "https://www.linkedin.com/in/sparse/",
            &draft,
            Vec::new(),
            Vec::new(),
            Utc::now(),
        );
        assert_eq!(profile.public_identifier.as_deref(), Some("sparse"));
        assert!(profile.full_name.is_none());
        assert!(profile.meta.sections_populated.is_empty());
        assert_eq!(profile.meta.completeness, 0.0);
    }
}
