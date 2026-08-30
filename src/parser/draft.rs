//! The intermediate representation shared by both profile models.
//!
//! Both parsers produce a `ProfileDraft`, so everything downstream
//! (enrichment, assembly, caching, serialisation) is written once.
//! Sections are typed fields, not stringly-typed maps, so section names are
//! checked at compile time.

use crate::domain::profile::{
    Certification, ContactInfo, Course, Education, Experience, Honor, ImageSet, Language, Location,
    NetworkInfo, OrganizationMembership, Patent, Project, Publication, Skill, TestScore,
    VolunteerExperience,
};

pub const SECTION_FIELDS: [&str; 13] = [
    "experience",
    "education",
    "skills",
    "certifications",
    "languages",
    "projects",
    "publications",
    "honors",
    "volunteering",
    "courses",
    "patents",
    "test_scores",
    "organizations",
];

/// Flat identity fields extracted from the member record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub full_name: Option<String>,
    pub headline: Option<String>,
    pub about: Option<String>,
    pub industry: Option<String>,
    pub pronouns: Option<String>,
    pub public_identifier: Option<String>,
    pub member_urn: Option<String>,
    pub profile_id: Option<String>,
    pub location: Option<Location>,
    pub profile_picture: Option<ImageSet>,
    pub background_picture: Option<ImageSet>,
}

/// The thirteen public profile sections.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sections {
    pub experience: Vec<Experience>,
    pub education: Vec<Education>,
    pub skills: Vec<Skill>,
    pub certifications: Vec<Certification>,
    pub languages: Vec<Language>,
    pub projects: Vec<Project>,
    pub publications: Vec<Publication>,
    pub honors: Vec<Honor>,
    pub volunteering: Vec<VolunteerExperience>,
    pub courses: Vec<Course>,
    pub patents: Vec<Patent>,
    pub test_scores: Vec<TestScore>,
    pub organizations: Vec<OrganizationMembership>,
}

impl Sections {
    /// Non-empty section names in contract order.
    pub fn populated(&self) -> Vec<&'static str> {
        SECTION_FIELDS
            .iter()
            .zip([
                !self.experience.is_empty(),
                !self.education.is_empty(),
                !self.skills.is_empty(),
                !self.certifications.is_empty(),
                !self.languages.is_empty(),
                !self.projects.is_empty(),
                !self.publications.is_empty(),
                !self.honors.is_empty(),
                !self.volunteering.is_empty(),
                !self.courses.is_empty(),
                !self.patents.is_empty(),
                !self.test_scores.is_empty(),
                !self.organizations.is_empty(),
            ])
            .filter(|(_, populated)| *populated)
            .map(|(name, _)| *name)
            .collect()
    }

    pub fn is_any_populated(&self) -> bool {
        !self.populated().is_empty()
    }

    /// Overwrite `skills` when a dedicated skills call provided endorsements.
    pub fn adopt_skills(&mut self, skills: Vec<Skill>) {
        if !skills.is_empty() {
            self.skills = skills;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileDraft {
    pub identity: Identity,
    pub sections: Sections,
    pub network: Option<NetworkInfo>,
    pub contact: Option<ContactInfo>,
    /// The strategy that produced this draft; "" before a parser runs.
    pub strategy: String,
}

impl ProfileDraft {
    pub fn is_populated(&self) -> bool {
        self.identity.full_name.is_some()
            || self.identity.headline.is_some()
            || self.identity.public_identifier.is_some()
            || self.sections.is_any_populated()
    }

    pub fn populated_sections(&self) -> Vec<&'static str> {
        self.sections.populated()
    }

    /// Prefer a dedicated skills call, which carries endorsement counts.
    pub fn adopt_skills(&mut self, skills: Vec<Skill>) {
        self.sections.adopt_skills(skills);
    }

    /// Combine the contact card with anything the profile call already gave
    /// us: neither source is authoritative on its own.
    pub fn merge_contact(&mut self, contact: Option<ContactInfo>) {
        let Some(contact) = contact else { return };
        match &mut self.contact {
            None => self.contact = Some(contact),
            Some(existing) => {
                existing.emails = merge_lists(&existing.emails, &contact.emails);
                existing.phone_numbers =
                    merge_lists(&existing.phone_numbers, &contact.phone_numbers);
                existing.websites = merge_lists(&existing.websites, &contact.websites);
                existing.twitter_handles =
                    merge_lists(&existing.twitter_handles, &contact.twitter_handles);
                existing.ims = merge_lists(&existing.ims, &contact.ims);
                if existing.birthday.is_none() {
                    existing.birthday = contact.birthday;
                }
                if existing.address.is_none() {
                    existing.address = contact.address;
                }
            }
        }
    }

    /// Fill only missing network fields from a dedicated network call.
    pub fn fill_network(&mut self, network: Option<NetworkInfo>) {
        let Some(network) = network else { return };
        match &mut self.network {
            None => self.network = Some(network),
            Some(existing) => {
                if existing.followers.is_none() {
                    existing.followers = network.followers;
                }
                if existing.connections.is_none() {
                    existing.connections = network.connections;
                }
                if existing.distance.is_none() {
                    existing.distance = network.distance;
                }
                if existing.following.is_none() {
                    existing.following = network.following;
                }
            }
        }
    }
}

fn merge_lists(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = existing.to_vec();
    for item in incoming {
        if !merged.contains(item) {
            merged.push(item.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populated_sections_are_in_contract_order() {
        let sections = Sections {
            patents: vec![Patent::default()],
            experience: vec![Experience::default()],
            ..Sections::default()
        };
        assert_eq!(sections.populated(), vec!["experience", "patents"]);
    }

    #[test]
    fn merge_contact_combines_and_dedupes() {
        let mut draft = ProfileDraft {
            contact: Some(ContactInfo {
                emails: vec!["a@x.com".into()],
                address: Some("Old".into()),
                ..Default::default()
            }),
            ..ProfileDraft::default()
        };
        draft.merge_contact(Some(ContactInfo {
            emails: vec!["a@x.com".into(), "b@x.com".into()],
            address: None,
            birthday: Some(crate::domain::profile::DateParts {
                year: None,
                month: Some(9),
                day: Some(19),
                iso: None,
                label: Some("19 Sep".into()),
            }),
            ..Default::default()
        }));
        let contact = draft.contact.unwrap();
        assert_eq!(contact.emails, vec!["a@x.com", "b@x.com"]);
        assert_eq!(contact.address.as_deref(), Some("Old"));
        assert_eq!(contact.birthday.unwrap().label.as_deref(), Some("19 Sep"));
    }

    #[test]
    fn fill_network_only_missing() {
        let mut draft = ProfileDraft {
            network: Some(NetworkInfo {
                followers: Some(7),
                connections: Some(3),
                distance: None,
                following: None,
            }),
            ..ProfileDraft::default()
        };
        draft.fill_network(Some(NetworkInfo {
            followers: Some(99),
            connections: None,
            distance: Some("DISTANCE_2".into()),
            following: Some(true),
        }));
        let network = draft.network.unwrap();
        assert_eq!(network.followers, Some(7));
        assert_eq!(network.connections, Some(3));
        assert_eq!(network.distance.as_deref(), Some("DISTANCE_2"));
        assert_eq!(network.following, Some(true));
    }
}
