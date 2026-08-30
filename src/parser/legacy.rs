//! Parsers for the legacy `profileView` projection and the core profile
//! record — kept as the final fallback strategy. Each section is an
//! independent function so a change in one LinkedIn view can only break that
//! one mapping.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::domain::profile::{
    Certification, Education, Experience, ImageSet, Location, NetworkInfo, OrganizationMembership,
    Patent, Project, Publication, Skill, TestScore, VolunteerExperience,
};

use super::common::*;
use super::draft::Identity;

pub fn parse_identity<'a>(
    sources: impl IntoIterator<Item = Option<&'a Map<String, Value>>>,
) -> Identity {
    let mut merged: Vec<&Map<String, Value>> = Vec::new();
    let mut minis: Vec<&Map<String, Value>> = Vec::new();
    for source in sources.into_iter().flatten() {
        if let Some(mini) = source.get("miniProfile").and_then(Value::as_object) {
            minis.push(mini);
        }
        merged.push(source);
    }

    let first_name = merged
        .iter()
        .find_map(|s| text_of(s, "firstName"))
        .or_else(|| minis.iter().find_map(|m| text_of(m, "firstName")));
    let last_name = merged
        .iter()
        .find_map(|s| text_of(s, "lastName"))
        .or_else(|| minis.iter().find_map(|m| text_of(m, "lastName")));

    let member_urn = normalise_from(
        merged.iter().copied(),
        &["objectUrn", "entityUrn"],
        "member",
    )
    .or_else(|| normalise_from(minis.iter().copied(), &["objectUrn", "entityUrn"], "member"));
    let profile_id = merged
        .iter()
        .find_map(|s| s.get("entityUrn"))
        .or_else(|| minis.iter().find_map(|m| m.get("entityUrn")))
        .and_then(Value::as_str)
        .and_then(urn_id)
        .map(str::to_string);

    Identity {
        first_name: first_name.clone(),
        last_name: last_name.clone(),
        full_name: joining(first_name, last_name),
        headline: first_text_from(&merged, &["headline", "occupation"]),
        about: first_text_from(&merged, &["summary"]),
        industry: first_text_from(&merged, &["industryName"]),
        pronouns: merged
            .iter()
            .find_map(|s| s.get("standardizedPronoun"))
            .or_else(|| merged.iter().find_map(|s| s.get("customPronoun")))
            .and_then(enum_label),
        public_identifier: first_text_from(&merged, &["publicIdentifier"]),
        member_urn,
        profile_id,
        location: location(&merged, &minis),
        profile_picture: picture(&merged, &minis, &["picture", "profilePicture"]),
        background_picture: picture(&merged, &minis, &["backgroundImage", "backgroundPicture"]),
    }
}

fn joining(first: Option<String>, last: Option<String>) -> Option<String> {
    let joined = [first, last]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn normalise_from<'a>(
    sources: impl IntoIterator<Item = &'a Map<String, Value>>,
    keys: &[&str],
    entity: &str,
) -> Option<String> {
    sources
        .into_iter()
        .find_map(|s| keys.iter().find_map(|k| s.get(*k)))
        .and_then(|v| normalise_urn(v, entity))
}

fn first_text_from(sources: &[&Map<String, Value>], keys: &[&str]) -> Option<String> {
    sources.iter().find_map(|s| first_text_of(s, keys))
}

fn location(merged: &[&Map<String, Value>], minis: &[&Map<String, Value>]) -> Option<Location> {
    let all: Vec<&Map<String, Value>> = merged
        .iter()
        .copied()
        .chain(minis.iter().copied())
        .collect();
    let country_code = all
        .iter()
        .find_map(|s| {
            let basic = s.get("location").and_then(Value::as_object)?;
            let basic = basic.get("basicLocation").and_then(Value::as_object)?;
            text_of(basic, "countryCode")
        })
        .map(|v| v.to_uppercase());
    let postal_code = all.iter().find_map(|s| {
        let basic = s.get("location").and_then(Value::as_object)?;
        let basic = basic.get("basicLocation").and_then(Value::as_object)?;
        text_of(basic, "postalCode")
    });
    let location = Location {
        label: all
            .iter()
            .find_map(|s| first_text_of(s, &["geoLocationName", "locationName", "geoCountryName"])),
        city: all.iter().find_map(|s| text_of(s, "city")),
        country_code,
        geo_urn: all
            .iter()
            .find_map(|s| {
                let geo = s.get("geoLocation").and_then(Value::as_object)?;
                geo.get("geoUrn").or_else(|| s.get("geoUrn"))
            })
            .and_then(|v| normalise_urn(v, "geo")),
        postal_code,
    };
    if location.label.is_some()
        || location.city.is_some()
        || location.country_code.is_some()
        || location.geo_urn.is_some()
        || location.postal_code.is_some()
    {
        Some(location)
    } else {
        None
    }
}

fn picture(
    merged: &[&Map<String, Value>],
    minis: &[&Map<String, Value>],
    keys: &[&str],
) -> Option<ImageSet> {
    for source in merged {
        for key in keys {
            if let Some(image) = image(source.get(*key)) {
                return Some(image);
            }
        }
    }
    for mini in minis {
        for key in keys {
            if let Some(image) = image(mini.get(*key)) {
                return Some(image);
            }
        }
    }
    None
}

fn image(node: Option<&Value>) -> Option<ImageSet> {
    parse_vector_image(node?)
}

// --------------------------------------------------------------------- sections

pub fn parse_experience(profile_view: &Map<String, Value>, now: (i64, i64)) -> Vec<Experience> {
    elements(profile_view, "positionView")
        .into_iter()
        .map(|element| Experience {
            title: text_of(element, "title"),
            employment_type: element.get("employmentType").and_then(enum_label),
            company: parse_company(&element.get("company").cloned().unwrap_or_else(|| {
                Value::Object(Map::from_iter([
                    (
                        "companyName".to_string(),
                        element.get("companyName").cloned().unwrap_or(Value::Null),
                    ),
                    (
                        "companyUrn".to_string(),
                        element.get("companyUrn").cloned().unwrap_or(Value::Null),
                    ),
                ]))
            })),
            location: first_text_of(element, &["locationName", "geoLocationName"]),
            description: text_of(element, "description"),
            dates: parse_time_period(element.get("timePeriod").unwrap_or(&Value::Null), true, now),
            skills: Vec::new(),
        })
        .collect()
}

pub fn parse_education(profile_view: &Map<String, Value>, now: (i64, i64)) -> Vec<Education> {
    elements(profile_view, "educationView")
        .into_iter()
        .map(|element| Education {
            school: parse_school(
                element.get("school").unwrap_or(&Value::Null),
                text_of(element, "schoolName"),
            ),
            degree: text_of(element, "degreeName"),
            field_of_study: text_of(element, "fieldOfStudy"),
            grade: text_of(element, "grade"),
            activities: text_of(element, "activities"),
            description: text_of(element, "description"),
            dates: parse_time_period(element.get("timePeriod").unwrap_or(&Value::Null), true, now),
        })
        .collect()
}

pub fn parse_skills(profile_view: &Map<String, Value>) -> Vec<Skill> {
    parse_skill_elements(elements(profile_view, "skillView"))
}

pub fn parse_skill_elements(items: Vec<&Map<String, Value>>) -> Vec<Skill> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut skills = Vec::new();
    for element in items {
        let Some(name) = text_of(element, "name") else {
            continue;
        };
        if !seen.insert(name.to_lowercase()) {
            continue;
        }
        let endorsement = element
            .get("endorsementCount")
            .filter(|v| integer(v) != Some(0))
            .or_else(|| element.get("numEndorsements"));
        skills.push(Skill {
            name,
            endorsement_count: integer(endorsement.unwrap_or(&Value::Null)),
        });
    }
    skills
}

pub fn parse_certifications(
    profile_view: &Map<String, Value>,
    now: (i64, i64),
) -> Vec<Certification> {
    elements(profile_view, "certificationView")
        .into_iter()
        .map(|element| Certification {
            name: text_of(element, "name"),
            authority: text_of(element, "authority"),
            license_number: text_of(element, "licenseNumber"),
            url: text_of(element, "url"),
            company: element.get("company").and_then(parse_company),
            dates: parse_time_period(
                element.get("timePeriod").unwrap_or(&Value::Null),
                false,
                now,
            ),
        })
        .collect()
}

pub fn parse_languages(profile_view: &Map<String, Value>) -> Vec<crate::domain::profile::Language> {
    elements(profile_view, "languageView")
        .into_iter()
        .map(|element| crate::domain::profile::Language {
            name: text_of(element, "name"),
            proficiency: element.get("proficiency").and_then(enum_label),
        })
        .collect()
}

pub fn parse_projects(profile_view: &Map<String, Value>, now: (i64, i64)) -> Vec<Project> {
    elements(profile_view, "projectView")
        .into_iter()
        .map(|element| Project {
            title: text_of(element, "title"),
            description: text_of(element, "description"),
            url: text_of(element, "url"),
            dates: parse_time_period(element.get("timePeriod").unwrap_or(&Value::Null), true, now),
            contributors: contributor_names(element.get("members").unwrap_or(&Value::Null)),
        })
        .collect()
}

pub fn parse_publications(profile_view: &Map<String, Value>) -> Vec<Publication> {
    elements(profile_view, "publicationView")
        .into_iter()
        .map(|element| Publication {
            name: text_of(element, "name"),
            publisher: text_of(element, "publisher"),
            description: text_of(element, "description"),
            url: text_of(element, "url"),
            published_on: element.get("date").and_then(parse_date),
            authors: contributor_names(element.get("authors").unwrap_or(&Value::Null)),
        })
        .collect()
}

pub fn parse_honors(profile_view: &Map<String, Value>) -> Vec<crate::domain::profile::Honor> {
    elements(profile_view, "honorView")
        .into_iter()
        .map(|element| crate::domain::profile::Honor {
            title: text_of(element, "title"),
            issuer: text_of(element, "issuer"),
            description: text_of(element, "description"),
            issued_on: element.get("issueDate").and_then(parse_date),
        })
        .collect()
}

pub fn parse_volunteering(
    profile_view: &Map<String, Value>,
    now: (i64, i64),
) -> Vec<VolunteerExperience> {
    elements(profile_view, "volunteerExperienceView")
        .into_iter()
        .map(|element| VolunteerExperience {
            role: text_of(element, "role"),
            organization: text_of(element, "companyName"),
            cause: element.get("cause").and_then(enum_label),
            description: text_of(element, "description"),
            dates: parse_time_period(element.get("timePeriod").unwrap_or(&Value::Null), true, now),
        })
        .collect()
}

pub fn parse_courses(profile_view: &Map<String, Value>) -> Vec<crate::domain::profile::Course> {
    elements(profile_view, "courseView")
        .into_iter()
        .map(|element| crate::domain::profile::Course {
            name: text_of(element, "name"),
            number: text_of(element, "number"),
        })
        .collect()
}

pub fn parse_patents(profile_view: &Map<String, Value>) -> Vec<Patent> {
    elements(profile_view, "patentView")
        .into_iter()
        .map(|element| Patent {
            title: text_of(element, "title"),
            number: first_text_of(element, &["number", "applicationNumber"]),
            description: text_of(element, "description"),
            url: text_of(element, "url"),
            issued_on: element
                .get("issueDate")
                .or_else(|| element.get("filingDate"))
                .and_then(parse_date),
            pending: element.get("pending").and_then(Value::as_bool),
        })
        .collect()
}

pub fn parse_test_scores(profile_view: &Map<String, Value>) -> Vec<TestScore> {
    elements(profile_view, "testScoreView")
        .into_iter()
        .map(|element| TestScore {
            name: text_of(element, "name"),
            score: text_of(element, "score"),
            description: text_of(element, "description"),
            taken_on: element.get("date").and_then(parse_date),
        })
        .collect()
}

pub fn parse_organizations(
    profile_view: &Map<String, Value>,
    now: (i64, i64),
) -> Vec<OrganizationMembership> {
    elements(profile_view, "organizationView")
        .into_iter()
        .map(|element| OrganizationMembership {
            name: text_of(element, "name"),
            position: text_of(element, "position"),
            description: text_of(element, "description"),
            dates: parse_time_period(element.get("timePeriod").unwrap_or(&Value::Null), true, now),
        })
        .collect()
}

pub fn parse_network(payload: &Map<String, Value>) -> Option<NetworkInfo> {
    let raw_distance = payload.get("distance");
    let distance_value = match raw_distance {
        Some(Value::Object(o)) => o.get("value").unwrap_or(raw_distance.unwrap()),
        _ => raw_distance.unwrap_or(&Value::Null),
    };
    let network = NetworkInfo {
        followers: integer(payload.get("followersCount").unwrap_or(&Value::Null)),
        connections: integer(payload.get("connectionsCount").unwrap_or(&Value::Null)),
        distance: text(distance_value),
        following: payload.get("following").and_then(Value::as_bool),
    };
    if network.followers.is_some()
        || network.connections.is_some()
        || network.distance.is_some()
        || network.following.is_some()
    {
        Some(network)
    } else {
        None
    }
}

fn contributor_names(value: &Value) -> Vec<String> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let member = item.get("member").and_then(Value::as_object);
        let name = text_of(item, "name").unwrap_or_else(|| {
            let first = member
                .and_then(|m| text_of(m, "firstName"))
                .unwrap_or_default();
            let last = member
                .and_then(|m| text_of(m, "lastName"))
                .unwrap_or_default();
            [first, last]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        });
        if !name.is_empty() {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identity_merges_miniprofile() {
        let core = json!({
            "firstName": "Ada",
            "lastName": "Lovelace",
            "headline": "Engineer",
            "publicIdentifier": "adalovelace",
            "objectUrn": "urn:li:member:987654321",
            "geoLocationName": "London",
            "miniProfile": {
                "picture": {"rootUrl": "https://x/", "artifacts": [{"fileIdentifyingUrlPathSegment": "p.jpg"}]},
            },
        });
        let identity = parse_identity([Some(core.as_object().unwrap()), None]);
        assert_eq!(identity.full_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(
            identity.member_urn.as_deref(),
            Some("urn:li:member:987654321")
        );
        assert_eq!(identity.location.unwrap().label.as_deref(), Some("London"));
        assert!(identity.profile_picture.is_some());
        assert_eq!(parse_identity([None, None]), Identity::default());
    }

    #[test]
    fn no_collection_no_crash() {
        let empty = json!({"profile": {"firstName": "X"}});
        let view = empty.as_object().unwrap();
        assert!(parse_experience(view, (2026, 8)).is_empty());
        assert!(parse_network(json!({}).as_object().unwrap()).is_none());
        assert!(parse_skills(view).is_empty());
    }
}
