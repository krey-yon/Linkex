//! Parsers for LinkedIn's current profile model ("Dash").
//!
//! Two envelopes are handled: the **embedded** decorated REST collection
//! (one element nesting `profile<Section>.elements[]`) and the **normalised**
//! GraphQL envelope (a flat `included` array referenced by URN). Both are
//! reduced to the same intermediate form (a root member record plus named
//! element collections) before they are mapped, so the two envelopes cannot
//! drift apart.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::domain::profile::{
    Certification, ContactInfo, Course, Education, Experience, Honor, ImageSet, Language, Location,
    NetworkInfo, Organization, OrganizationMembership, Patent, Project, Publication, Skill,
    TestScore, VolunteerExperience,
};

use super::common::*;
use super::draft::{Identity, Sections};
use super::normalized::{EntityIndex, find_vector_image};

/// Type suffix pairs for each collection: (embedded key, normalised suffixes).
const SECTION_SOURCES: [(&str, &[&str]); 14] = [
    (
        "profilePositionGroups",
        &["ProfilePositionGroup", "PositionGroup"],
    ),
    ("profilePositions", &["ProfilePosition", "Position"]),
    ("profileEducations", &["ProfileEducation", "Education"]),
    ("profileSkills", &["ProfileSkill", "Skill"]),
    (
        "profileCertifications",
        &["ProfileCertification", "Certification"],
    ),
    ("profileLanguages", &["ProfileLanguage", "Language"]),
    ("profileProjects", &["ProfileProject", "Project"]),
    (
        "profilePublications",
        &["ProfilePublication", "Publication"],
    ),
    ("profileHonors", &["ProfileHonor", "Honor"]),
    (
        "profileVolunteerExperiences",
        &["ProfileVolunteerExperience", "VolunteerExperience"],
    ),
    ("profileCourses", &["ProfileCourse", "Course"]),
    ("profilePatents", &["ProfilePatent", "Patent"]),
    ("profileTestScores", &["ProfileTestScore", "TestScore"]),
    (
        "profileOrganizations",
        &["ProfileOrganization", "Organization"],
    ),
];

enum Env<'a> {
    Embedded,
    Normalized(EntityIndex<'a>),
}

impl<'a> Env<'a> {
    fn field<'v>(&self, entity: &'v Map<String, Value>, key: &str) -> Option<&'v Value>
    where
        'a: 'v,
    {
        match self {
            Env::Embedded => entity.get(key),
            Env::Normalized(index) => index.field(Some(entity), key),
        }
    }
}

/// The intermediate extraction result: root member plus named element
/// collections (embedded envelope) or a type index (normalised envelope).
struct Extracted<'a> {
    root: &'a Map<String, Value>,
    collections: HashMap<&'static str, Vec<&'a Map<String, Value>>>,
    env: Env<'a>,
}

pub struct DashProfileDocument {
    pub identity: Identity,
    pub sections: Sections,
    pub network: Option<NetworkInfo>,
    pub contact: Option<ContactInfo>,
}

/// Maps a Dash profile response onto the public schema. `now` is
/// `(year, month)` used for open-ended durations. Returns `None` when the
/// payload carries no member record.
pub fn parse_dash_profile(payload: &Value, now: (i64, i64)) -> Option<DashProfileDocument> {
    let extracted = extract(payload)?;
    let env = &extracted.env;

    let positions = flatten_positions(&extracted.collections, env);
    let sections = Sections {
        experience: experience(&positions, now),
        education: education(entities_of(&extracted, "profileEducations"), now),
        skills: skills(entities_of(&extracted, "profileSkills")),
        certifications: certifications(entities_of(&extracted, "profileCertifications"), now),
        languages: languages(entities_of(&extracted, "profileLanguages")),
        projects: projects(entities_of(&extracted, "profileProjects"), now),
        publications: publications(entities_of(&extracted, "profilePublications")),
        honors: honors(entities_of(&extracted, "profileHonors")),
        volunteering: volunteering(entities_of(&extracted, "profileVolunteerExperiences"), now),
        courses: courses(entities_of(&extracted, "profileCourses")),
        patents: patents(entities_of(&extracted, "profilePatents")),
        test_scores: test_scores(entities_of(&extracted, "profileTestScores")),
        organizations: organizations(entities_of(&extracted, "profileOrganizations"), now),
    };

    Some(DashProfileDocument {
        identity: identity(extracted.root, env),
        sections,
        network: network(extracted.root, env),
        contact: contact_from_profile(extracted.root),
    })
}

// ------------------------------------------------------------------ envelopes

fn extract(payload: &Value) -> Option<Extracted<'_>> {
    let obj = payload.as_object()?;
    if obj.contains_key("included") || obj.contains_key("data") {
        let index = EntityIndex::new(payload)?;
        let root = normalised_root(payload, &index)?;
        Some(Extracted {
            root,
            collections: HashMap::new(),
            env: Env::Normalized(index),
        })
    } else {
        let root = embedded_root(payload)?;
        let collections = SECTION_SOURCES
            .iter()
            .map(|(key, _)| {
                let items = root.get(*key).map(elements_from_value).unwrap_or_default();
                (*key, items)
            })
            .collect();
        Some(Extracted {
            root,
            collections,
            env: Env::Embedded,
        })
    }
}

fn elements_from_value(value: &Value) -> Vec<&Map<String, Value>> {
    match value {
        Value::Object(o) => o
            .get("elements")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(|v| v.as_object()).collect())
            .unwrap_or_default(),
        Value::Array(arr) => arr.iter().filter_map(|v| v.as_object()).collect(),
        _ => Vec::new(),
    }
}

fn embedded_root(payload: &Value) -> Option<&Map<String, Value>> {
    let obj = payload.as_object()?;
    if let Some(items) = obj.get("elements").and_then(Value::as_array) {
        for candidate in items {
            if let Some(candidate) = candidate.as_object()
                && looks_like_member(candidate)
            {
                return Some(candidate);
            }
        }
    }
    if looks_like_member(obj) {
        Some(obj)
    } else {
        None
    }
}

fn normalised_root<'a>(
    _payload: &'a Value,
    index: &EntityIndex<'a>,
) -> Option<&'a Map<String, Value>> {
    let candidates: Vec<&Map<String, Value>> = index
        .of_type(&["profile.Profile", "Profile"])
        .into_iter()
        .filter(|e| looks_like_member(e))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pointer = index
        .data()
        .and_then(|d| d.get("*elements").or_else(|| d.get("*profile")))
        .and_then(|v| match v {
            Value::Array(items) => items.first(),
            Value::String(_) => Some(v),
            _ => None,
        })
        .and_then(Value::as_str);
    if let Some(pointer) = pointer {
        for candidate in &candidates {
            if candidate.get("entityUrn").and_then(Value::as_str) == Some(pointer) {
                return Some(candidate);
            }
        }
    }
    candidates.into_iter().max_by_key(|e| e.len())
}

fn looks_like_member(entity: &Map<String, Value>) -> bool {
    ["firstName", "lastName", "publicIdentifier", "headline"]
        .iter()
        .any(|key| entity.contains_key(*key))
}

// --------------------------------------------------------- collection access

/// Entities of one section: from the embedded root's collection, or by type
/// suffix when the envelope is normalised.
fn entities_of<'a>(extracted: &Extracted<'a>, key: &'static str) -> Vec<&'a Map<String, Value>> {
    match &extracted.env {
        Env::Normalized(index) => {
            let (_, suffixes) = SECTION_SOURCES
                .iter()
                .find(|(k, _)| *k == key)
                .expect("known section key");
            index.of_type(suffixes)
        }
        Env::Embedded => extracted.collections.get(key).cloned().unwrap_or_default(),
    }
}

// ------------------------------------------------------------------- identity

fn identity(root: &Map<String, Value>, env: &Env<'_>) -> Identity {
    let first_name = text_of(root, "firstName");
    let last_name = text_of(root, "lastName");
    let industry = env
        .field(root, "industry")
        .and_then(named)
        .or_else(|| text_of(root, "industryName"));

    Identity {
        first_name: first_name.clone(),
        last_name: last_name.clone(),
        full_name: joining(first_name, last_name),
        headline: text_of(root, "headline"),
        about: text_of(root, "summary"),
        industry,
        pronouns: env
            .field(root, "standardizedPronoun")
            .or_else(|| env.field(root, "customPronoun"))
            .and_then(enum_label),
        public_identifier: text_of(root, "publicIdentifier"),
        member_urn: env
            .field(root, "objectUrn")
            .or_else(|| root.get("entityUrn"))
            .and_then(|v| normalise_urn(v, "member")),
        profile_id: root
            .get("entityUrn")
            .and_then(Value::as_str)
            .and_then(urn_id)
            .map(str::to_string),
        location: locate(root, env),
        profile_picture: image(env.field(root, "profilePicture")),
        background_picture: image(env.field(root, "backgroundPicture")),
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

fn named(value: &Value) -> Option<String> {
    match value {
        Value::Object(o) => first_text_of(o, &["name", "defaultLocalizedName", "localizedName"]),
        _ => text(value),
    }
}

fn locate(root: &Map<String, Value>, env: &Env<'_>) -> Option<Location> {
    let geo_location = env.field(root, "geoLocation").and_then(Value::as_object);
    let geo = geo_location
        .and_then(|g| env.field(g, "geo"))
        .and_then(Value::as_object);
    let basic = match root.get("location") {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    };
    let country_code = basic
        .and_then(|b| text_of(b, "countryCode"))
        .map(|v| v.to_uppercase());
    let country_name = geo.and_then(|g| env.field(g, "country")).and_then(named);
    let label = geo
        .map(|g| Value::Object(g.clone()))
        .as_ref()
        .and_then(named)
        .or_else(|| first_text_of(root, &["geoLocationName", "locationName"]))
        .or(country_name);
    let city = geo
        .and_then(|g| text_of(g, "defaultLocalizedNameWithoutCountryName"))
        .or_else(|| text_of(root, "city"));
    let geo_urn = geo
        .and_then(|g| g.get("entityUrn"))
        .or_else(|| geo_location.and_then(|g| g.get("geoUrn")))
        .and_then(|v| normalise_urn(v, "geo"));

    let location = Location {
        label,
        city,
        country_code,
        geo_urn,
        postal_code: basic.and_then(|b| text_of(b, "postalCode")),
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

fn image(node: Option<&Value>) -> Option<ImageSet> {
    let vector = find_vector_image(node?, 0)?;
    parse_vector_image(&Value::Object(vector.clone()))
}

fn contact_from_profile(root: &Map<String, Value>) -> Option<ContactInfo> {
    let birthday = parse_date(root.get("birthDateOn")?)?;
    Some(ContactInfo {
        birthday: Some(birthday),
        ..Default::default()
    })
}

// ------------------------------------------------------------------ sections

fn experience(positions: &[Map<String, Value>], now: (i64, i64)) -> Vec<Experience> {
    let mut items: Vec<Experience> = positions
        .iter()
        .filter(|entity| {
            text_of(entity, "title").is_some() || text_of(entity, "companyName").is_some()
        })
        .map(|entity| Experience {
            title: text_of(entity, "title"),
            employment_type: {
                let raw = entity.get("employmentType").cloned().or_else(|| {
                    entity
                        .get("employmentTypeUrn")
                        .and_then(Value::as_str)
                        .and_then(urn_id)
                        .map(|s| Value::String(s.to_string()))
                });
                match raw {
                    Some(raw) => enum_label(&raw),
                    None => None,
                }
            },
            company: company(entity),
            location: first_text_of(entity, &["locationName", "geoLocationName"]),
            description: text_of(entity, "description"),
            dates: date_range(entity, now, true),
            skills: Vec::new(),
        })
        .collect();
    items.sort_by_key(experience_sort_key);
    items
}

fn experience_sort_key(item: &Experience) -> (u8, i64, i64) {
    match item
        .dates
        .as_ref()
        .and_then(|d| d.start.as_ref())
        .and_then(|s| s.year)
    {
        Some(year) => (
            0,
            -year,
            -(item
                .dates
                .as_ref()
                .and_then(|d| d.start.as_ref())
                .and_then(|s| s.month)
                .unwrap_or(0)),
        ),
        None => (1, 0, 0),
    }
}

fn company(entity: &Map<String, Value>) -> Option<Organization> {
    let company = match entity.get("company") {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    };
    let name = text_of(entity, "companyName")
        .or_else(|| company.and_then(|c| first_text_of(c, &["name", "defaultLocalizedName"])));
    let urn_value = company
        .and_then(|c| c.get("entityUrn"))
        .or_else(|| entity.get("companyUrn"))
        .and_then(|v| normalise_urn(v, "organization"));
    let url = company.and_then(|c| text_of(c, "url"));
    let universal_name = url.as_deref().and_then(universal_name_of);
    let logo = company
        .and_then(|c| c.get("logo"))
        .or_else(|| entity.get("logo"))
        .and_then(|v| image(Some(v)));

    if name.is_none() && url.is_none() && logo.is_none() && urn_value.is_none() {
        return None;
    }
    Some(Organization {
        name,
        urn: urn_value,
        universal_name: universal_name.clone(),
        linkedin_url: url
            .or_else(|| universal_name.map(|u| format!("https://www.linkedin.com/company/{u}"))),
        logo,
    })
}

fn universal_name_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("/company/")?;
    let name = rest
        .trim_matches('/')
        .split('/')
        .next()?
        .split('?')
        .next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn school(entity: &Map<String, Value>) -> Option<Organization> {
    let school = match entity.get("school") {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    };
    let name = text_of(entity, "schoolName")
        .or_else(|| school.and_then(|s| first_text_of(s, &["name", "defaultLocalizedName"])));
    let urn_value = school
        .and_then(|s| s.get("entityUrn"))
        .or_else(|| entity.get("schoolUrn"))
        .and_then(|v| normalise_urn(v, "school"));
    let logo = school
        .and_then(|s| s.get("logo"))
        .or_else(|| entity.get("logo"))
        .and_then(|v| image(Some(v)));
    if name.is_none() && logo.is_none() && urn_value.is_none() {
        return None;
    }
    Some(Organization {
        name,
        urn: urn_value,
        universal_name: None,
        linkedin_url: None,
        logo,
    })
}

fn date_range(
    entity: &Map<String, Value>,
    now: (i64, i64),
    current: bool,
) -> Option<crate::domain::profile::DateRange> {
    let raw = entity
        .get("dateRange")
        .or_else(|| entity.get("timePeriod"))?;
    let raw = raw.as_object()?;
    let mapped = Value::Object(Map::from_iter([
        (
            "startDate".to_string(),
            raw.get("start")
                .or_else(|| raw.get("startDate"))
                .unwrap_or(&Value::Null)
                .clone(),
        ),
        (
            "endDate".to_string(),
            raw.get("end")
                .or_else(|| raw.get("endDate"))
                .unwrap_or(&Value::Null)
                .clone(),
        ),
    ]));
    parse_time_period(&mapped, current, now)
}

fn single_date(
    entity: &Map<String, Value>,
    keys: &[&str],
) -> Option<crate::domain::profile::DateParts> {
    for key in keys {
        if let Some(parsed) = parse_date(entity.get(*key).unwrap_or(&Value::Null)) {
            return Some(parsed);
        }
    }
    if let Some(Value::Object(range)) = entity.get("dateRange")
        && let Some(start) = range.get("start").or_else(|| range.get("startDate"))
    {
        return parse_date(start);
    }
    None
}

pub fn education(items: Vec<&Map<String, Value>>, now: (i64, i64)) -> Vec<Education> {
    let mut items: Vec<Education> = items
        .into_iter()
        .filter(|e| {
            ["schoolName", "school", "degreeName", "schoolUrn"]
                .iter()
                .any(|key| e.contains_key(*key))
        })
        .map(|entity| Education {
            school: school(entity),
            degree: text_of(entity, "degreeName"),
            field_of_study: text_of(entity, "fieldOfStudy"),
            grade: text_of(entity, "grade"),
            activities: text_of(entity, "activities"),
            description: text_of(entity, "description"),
            dates: date_range(entity, now, true),
        })
        .collect();
    items.sort_by_key(education_sort_key);
    items
}

fn education_sort_key(item: &Education) -> (u8, i64, i64) {
    match item
        .dates
        .as_ref()
        .and_then(|d| d.start.as_ref())
        .and_then(|s| s.year)
    {
        Some(year) => (
            0,
            -year,
            -(item
                .dates
                .as_ref()
                .and_then(|d| d.start.as_ref())
                .and_then(|s| s.month)
                .unwrap_or(0)),
        ),
        None => (1, 0, 0),
    }
}

pub fn skills(items: Vec<&Map<String, Value>>) -> Vec<Skill> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut skills = Vec::new();
    for entity in items {
        let Some(name) = text_of(entity, "name") else {
            continue;
        };
        if !seen.insert(name.to_lowercase()) {
            continue;
        }
        // Python's `or` treats a zero count as absent; mirror that.
        let endorsement = entity
            .get("endorsementCount")
            .filter(|v| integer(v) != Some(0))
            .or_else(|| entity.get("numEndorsements"));
        skills.push(Skill {
            name,
            endorsement_count: integer(endorsement.unwrap_or(&Value::Null)),
        });
    }
    skills
}

fn certifications(items: Vec<&Map<String, Value>>, now: (i64, i64)) -> Vec<Certification> {
    items
        .into_iter()
        .filter(|e| text_of(e, "name").is_some())
        .map(|entity| Certification {
            name: text_of(entity, "name"),
            authority: text_of(entity, "authority"),
            license_number: text_of(entity, "licenseNumber"),
            url: text_of(entity, "url"),
            company: company(entity),
            dates: date_range(entity, now, false),
        })
        .collect()
}

fn languages(items: Vec<&Map<String, Value>>) -> Vec<Language> {
    items
        .into_iter()
        .filter(|e| text_of(e, "name").is_some())
        .map(|entity| Language {
            name: text_of(entity, "name"),
            proficiency: entity.get("proficiency").and_then(enum_label),
        })
        .collect()
}

pub fn projects(items: Vec<&Map<String, Value>>, now: (i64, i64)) -> Vec<Project> {
    items
        .into_iter()
        .filter(|e| first_text_of(e, &["title", "name"]).is_some())
        .map(|entity| Project {
            title: first_text_of(entity, &["title", "name"]),
            description: text_of(entity, "description"),
            url: text_of(entity, "url"),
            dates: date_range(entity, now, true),
            contributors: contributors(entity, &["contributors", "members"]),
        })
        .collect()
}

pub fn publications(items: Vec<&Map<String, Value>>) -> Vec<Publication> {
    items
        .into_iter()
        .filter(|e| first_text_of(e, &["name", "title"]).is_some())
        .map(|entity| Publication {
            name: first_text_of(entity, &["name", "title"]),
            publisher: text_of(entity, "publisher"),
            description: text_of(entity, "description"),
            url: text_of(entity, "url"),
            published_on: single_date(entity, &["publishedOn", "date"]),
            authors: contributors(entity, &["authors", "contributors"]),
        })
        .collect()
}

fn honors(items: Vec<&Map<String, Value>>) -> Vec<Honor> {
    items
        .into_iter()
        .filter(|e| first_text_of(e, &["title", "name"]).is_some())
        .map(|entity| Honor {
            title: first_text_of(entity, &["title", "name"]),
            issuer: text_of(entity, "issuer"),
            description: text_of(entity, "description"),
            issued_on: single_date(entity, &["issuedOn", "issueDate"]),
        })
        .collect()
}

fn volunteering(items: Vec<&Map<String, Value>>, now: (i64, i64)) -> Vec<VolunteerExperience> {
    items
        .into_iter()
        .map(|entity| {
            let company = company(entity);
            VolunteerExperience {
                role: text_of(entity, "role"),
                organization: company
                    .as_ref()
                    .and_then(|c| c.name.clone())
                    .or_else(|| text_of(entity, "companyName")),
                cause: entity.get("cause").and_then(enum_label),
                description: text_of(entity, "description"),
                dates: date_range(entity, now, true),
            }
        })
        .collect()
}

fn courses(items: Vec<&Map<String, Value>>) -> Vec<Course> {
    items
        .into_iter()
        .filter(|e| text_of(e, "name").is_some())
        .map(|entity| Course {
            name: text_of(entity, "name"),
            number: text_of(entity, "number"),
        })
        .collect()
}

fn patents(items: Vec<&Map<String, Value>>) -> Vec<Patent> {
    items
        .into_iter()
        .map(|entity| Patent {
            title: text_of(entity, "title"),
            number: first_text_of(entity, &["number", "applicationNumber"]),
            description: text_of(entity, "description"),
            url: text_of(entity, "url"),
            issued_on: single_date(entity, &["issuedOn", "issueDate", "filingDate"]),
            pending: entity.get("pending").and_then(Value::as_bool),
        })
        .collect()
}

fn test_scores(items: Vec<&Map<String, Value>>) -> Vec<TestScore> {
    items
        .into_iter()
        .filter(|e| text_of(e, "name").is_some())
        .map(|entity| TestScore {
            name: text_of(entity, "name"),
            score: text_of(entity, "score"),
            description: text_of(entity, "description"),
            taken_on: single_date(entity, &["takenOn", "date"]),
        })
        .collect()
}

fn organizations(items: Vec<&Map<String, Value>>, now: (i64, i64)) -> Vec<OrganizationMembership> {
    items
        .into_iter()
        .filter(|e| text_of(e, "name").is_some())
        .map(|entity| OrganizationMembership {
            name: text_of(entity, "name"),
            position: text_of(entity, "position"),
            description: text_of(entity, "description"),
            dates: date_range(entity, now, true),
        })
        .collect()
}

fn network(root: &Map<String, Value>, env: &Env<'_>) -> Option<NetworkInfo> {
    let followers_direct = integer(root.get("followerCount").unwrap_or(&Value::Null));
    let mut followers = followers_direct;
    let mut following = None;

    if let Some(Value::Object(state)) = env.field(root, "followingState") {
        followers =
            followers.or_else(|| integer(state.get("followerCount").unwrap_or(&Value::Null)));
        following = state.get("following").and_then(Value::as_bool);
    }
    if let Env::Normalized(index) = env
        && let Some(entity) = index.first_of_type(&["FollowingState"])
    {
        followers =
            followers.or_else(|| integer(entity.get("followerCount").unwrap_or(&Value::Null)));
    }

    let network = NetworkInfo {
        followers,
        connections: integer(root.get("connectionsCount").unwrap_or(&Value::Null)),
        distance: None,
        following,
    };
    if network.followers.is_some() || network.connections.is_some() || network.following.is_some() {
        Some(network)
    } else {
        None
    }
}

fn contributors(entity: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for key in keys {
        let items = match entity.get(*key) {
            Some(Value::Object(o)) => match o.get("elements") {
                Some(Value::Array(items)) => items,
                _ => continue,
            },
            Some(Value::Array(items)) => items,
            _ => continue,
        };
        for item in items {
            let Some(item) = item.as_object() else {
                continue;
            };
            let profile = item
                .get("profile")
                .and_then(Value::as_object)
                .or(Some(item));
            let name = text_of(item, "name").unwrap_or_else(|| {
                joining(
                    profile.and_then(|p| text_of(p, "firstName")),
                    profile.and_then(|p| text_of(p, "lastName")),
                )
                .unwrap_or_default()
            });
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

// ------------------------------------------------------------------ positions

/// Positions live inside position groups; a flat list is the useful shape.
/// Groups are unwrapped and the group's company details are inherited when a
/// role omits them. Merged maps are owned (they combine group + position),
/// mirroring the Python implementation.
fn flatten_positions<'a>(
    collections: &HashMap<&'static str, Vec<&'a Map<String, Value>>>,
    env: &Env<'a>,
) -> Vec<Map<String, Value>> {
    let standalone = match env {
        // The normalised envelope has no embedded collections; resolve both
        // standalone positions and position groups from the entity index.
        Env::Normalized(index) => index.of_type(&["ProfilePosition", "Position"]),
        Env::Embedded => collections
            .get("profilePositions")
            .cloned()
            .unwrap_or_default(),
    };
    let mut positions: Vec<Map<String, Value>> = standalone.into_iter().cloned().collect();

    let position_urms = |p: &Map<String, Value>| {
        p.get("entityUrn")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let mut seen: HashSet<Option<String>> = positions.iter().map(position_urms).collect();

    let groups: Vec<&Map<String, Value>> = match env {
        Env::Normalized(index) => index.of_type(&["ProfilePositionGroup", "PositionGroup"]),
        Env::Embedded => collections
            .get("profilePositionGroups")
            .cloned()
            .unwrap_or_default(),
    };
    for group in groups {
        let group_company_name = text_of(group, "companyName");
        let group_company_urn = group.get("companyUrn").cloned();
        let group_company = group.get("company").cloned();
        for position in group_positions(group, env) {
            if !seen.insert(
                position
                    .get("entityUrn")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ) {
                continue;
            }
            let mut merged = position.clone();
            if merged.get("companyName").is_none() && group_company_name.is_some() {
                merged.insert(
                    "companyName".to_string(),
                    Value::String(group_company_name.clone().unwrap()),
                );
            }
            if merged.get("companyUrn").is_none()
                && let Some(value) = &group_company_urn
            {
                merged.insert("companyUrn".to_string(), value.clone());
            }
            if merged.get("company").is_none()
                && let Some(value) = &group_company
            {
                merged.insert("company".to_string(), value.clone());
            }
            positions.push(merged);
        }
    }
    positions
}

fn group_positions<'a>(
    group: &'a Map<String, Value>,
    env: &Env<'a>,
) -> Vec<&'a Map<String, Value>> {
    match env {
        Env::Normalized(index) => {
            index.elements_named(Some(group), "profilePositionInPositionGroup")
        }
        Env::Embedded => elements_from_value(
            group
                .get("profilePositionInPositionGroup")
                .unwrap_or(&Value::Null),
        ),
    }
}
