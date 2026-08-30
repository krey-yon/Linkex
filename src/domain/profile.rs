//! The public response schema.
//!
//! Ported from the Python service: snake_case keys, explicit `null` for absent
//! optional fields, `[]` for absent list fields. Field order matches the Python
//! model declaration order so byte-level JSON comparison is meaningful.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DateParts {
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
    pub iso: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DateRange {
    pub start: Option<DateParts>,
    pub end: Option<DateParts>,
    #[serde(default)]
    pub is_current: bool,
    pub duration_months: Option<i64>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRendition {
    pub url: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSet {
    pub url: String,
    #[serde(default)]
    pub renditions: Vec<ImageRendition>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Location {
    pub label: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub geo_urn: Option<String>,
    pub postal_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Organization {
    pub name: Option<String>,
    pub urn: Option<String>,
    pub universal_name: Option<String>,
    pub linkedin_url: Option<String>,
    pub logo: Option<ImageSet>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Experience {
    pub title: Option<String>,
    pub employment_type: Option<String>,
    pub company: Option<Organization>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub dates: Option<DateRange>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Education {
    pub school: Option<Organization>,
    pub degree: Option<String>,
    pub field_of_study: Option<String>,
    pub grade: Option<String>,
    pub activities: Option<String>,
    pub description: Option<String>,
    pub dates: Option<DateRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub endorsement_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Certification {
    pub name: Option<String>,
    pub authority: Option<String>,
    pub license_number: Option<String>,
    pub url: Option<String>,
    pub company: Option<Organization>,
    pub dates: Option<DateRange>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Language {
    pub name: Option<String>,
    pub proficiency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Project {
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub dates: Option<DateRange>,
    #[serde(default)]
    pub contributors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Publication {
    pub name: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub published_on: Option<DateParts>,
    #[serde(default)]
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Honor {
    pub title: Option<String>,
    pub issuer: Option<String>,
    pub description: Option<String>,
    pub issued_on: Option<DateParts>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VolunteerExperience {
    pub role: Option<String>,
    pub organization: Option<String>,
    pub cause: Option<String>,
    pub description: Option<String>,
    pub dates: Option<DateRange>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Course {
    pub name: Option<String>,
    pub number: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Patent {
    pub title: Option<String>,
    pub number: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub issued_on: Option<DateParts>,
    pub pending: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TestScore {
    pub name: Option<String>,
    pub score: Option<String>,
    pub description: Option<String>,
    pub taken_on: Option<DateParts>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OrganizationMembership {
    pub name: Option<String>,
    pub position: Option<String>,
    pub description: Option<String>,
    pub dates: Option<DateRange>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub followers: Option<i64>,
    pub connections: Option<i64>,
    pub distance: Option<String>,
    pub following: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContactInfo {
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phone_numbers: Vec<String>,
    #[serde(default)]
    pub websites: Vec<String>,
    #[serde(default)]
    pub twitter_handles: Vec<String>,
    pub birthday: Option<DateParts>,
    pub address: Option<String>,
    #[serde(default)]
    pub ims: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCall {
    pub endpoint: String,
    pub status_code: i64,
    pub ok: bool,
    pub elapsed_ms: i64,
    #[serde(default = "default_attempts")]
    pub attempts: i64,
}

fn default_attempts() -> i64 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub fetched_at: DateTime<Utc>,
    #[serde(default)]
    pub sources: Vec<SourceCall>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub sections_populated: Vec<String>,
    #[serde(default)]
    pub completeness: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub profile_url: String,
    pub public_identifier: Option<String>,
    pub member_urn: Option<String>,
    pub profile_id: Option<String>,

    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub full_name: Option<String>,
    pub headline: Option<String>,
    pub about: Option<String>,
    pub industry: Option<String>,
    pub pronouns: Option<String>,
    pub location: Option<Location>,

    pub profile_picture: Option<ImageSet>,
    pub background_picture: Option<ImageSet>,

    pub network: Option<NetworkInfo>,
    pub contact: Option<ContactInfo>,

    #[serde(default)]
    pub experience: Vec<Experience>,
    #[serde(default)]
    pub education: Vec<Education>,
    #[serde(default)]
    pub skills: Vec<Skill>,
    #[serde(default)]
    pub certifications: Vec<Certification>,
    #[serde(default)]
    pub languages: Vec<Language>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub publications: Vec<Publication>,
    #[serde(default)]
    pub honors: Vec<Honor>,
    #[serde(default)]
    pub volunteering: Vec<VolunteerExperience>,
    #[serde(default)]
    pub courses: Vec<Course>,
    #[serde(default)]
    pub patents: Vec<Patent>,
    #[serde(default)]
    pub test_scores: Vec<TestScore>,
    #[serde(default)]
    pub organizations: Vec<OrganizationMembership>,

    pub meta: ProfileMeta,
}
