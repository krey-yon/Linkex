//! Parser for the profile contact card. LinkedIn only returns these fields
//! when the member shares them with the querying account, so an empty result
//! is a normal outcome rather than an error.

use serde_json::{Map, Value};

use crate::domain::profile::ContactInfo;

use super::common::*;

pub fn parse_contact_info(payload: &Value) -> Option<ContactInfo> {
    let payload = payload.as_object()?;

    let mut emails: Vec<String> = Vec::new();
    let mut phone_numbers: Vec<String> = Vec::new();
    let mut websites: Vec<String> = Vec::new();
    let mut twitter_handles: Vec<String> = Vec::new();
    let mut ims: Vec<String> = Vec::new();

    push_unique(&mut emails, text_of(payload, "emailAddress"));

    for item in list_of(payload.get("phoneNumbers")) {
        push_unique(&mut phone_numbers, first_text_of(item, &["number"]));
    }
    for item in list_of(payload.get("websites")) {
        push_unique(&mut websites, first_text_of(item, &["url"]));
    }
    for item in list_of(payload.get("twitterHandles")) {
        push_unique(&mut twitter_handles, first_text_of(item, &["name"]));
    }
    for item in list_of(payload.get("ims")) {
        push_unique(&mut ims, im_label(item));
    }

    let contact = ContactInfo {
        emails,
        phone_numbers,
        websites,
        twitter_handles,
        birthday: payload
            .get("birthDateOn")
            .or_else(|| payload.get("birthDate"))
            .and_then(parse_date),
        address: text_of(payload, "address"),
        ims,
    };

    let populated = !contact.emails.is_empty()
        || !contact.phone_numbers.is_empty()
        || !contact.websites.is_empty()
        || !contact.twitter_handles.is_empty()
        || !contact.ims.is_empty()
        || contact.birthday.is_some()
        || contact.address.is_some();
    if populated { Some(contact) } else { None }
}

fn list_of(value: Option<&Value>) -> Vec<&Map<String, Value>> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_object()).collect(),
        _ => Vec::new(),
    }
}

fn im_label(item: &Map<String, Value>) -> Option<String> {
    let provider = text_of(item, "provider");
    let handle = text_of(item, "id")?;
    match provider {
        Some(provider) => {
            let mut chars = provider.chars();
            let capitalized: String = chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>())
                .unwrap_or_default()
                .chars()
                .chain(chars)
                .collect();
            Some(format!("{capitalized}: {handle}"))
        }
        None => Some(handle),
    }
}

fn push_unique(list: &mut Vec<String>, value: Option<String>) {
    if let Some(value) = value {
        let lower = value.to_lowercase();
        if !list.iter().any(|existing| existing.to_lowercase() == lower) {
            list.push(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn contact_info_parsing() {
        let contact = parse_contact_info(&json!({
            "emailAddress": "ada@example.com",
            "phoneNumbers": [{"number": "+44 20 1234"}],
            "websites": [{"url": "https://ada.example"}],
            "twitterHandles": [{"name": "@Ada"}],
            "birthDateOn": {"month": 9, "day": 19},
            "ims": [{"provider": "skype", "id": "adalovelace"}],
        }))
        .unwrap();
        assert_eq!(contact.emails, vec!["ada@example.com"]);
        assert_eq!(contact.phone_numbers, vec!["+44 20 1234"]);
        assert_eq!(contact.twitter_handles, vec!["@Ada"]);
        assert_eq!(contact.birthday.unwrap().label.as_deref(), Some("19 Sep"));
        assert_eq!(contact.ims, vec!["Skype: adalovelace"]);
    }

    #[test]
    fn empty_contact_is_none() {
        assert!(parse_contact_info(&json!(null)).is_none());
        assert!(parse_contact_info(&json!({})).is_none());
    }

    #[test]
    fn malformed_items_do_not_panic() {
        assert!(
            parse_contact_info(&json!({"phoneNumbers": [7, null, "x"], "websites": "y"})).is_none()
        );
    }
}
