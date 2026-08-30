//! Shared defensive primitives for turning Voyager payloads into the public
//! schema. Every helper degrades to `None`/empty on missing, null, or
//! wrong-typed input — never panics, never indexes blindly.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::domain::profile::{DateParts, DateRange, ImageRendition, ImageSet, Organization};

pub fn text(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    let collapsed = s.replace("\r\n", "\n");

    let mut out = String::with_capacity(collapsed.len());
    let mut in_space = false;
    for ch in collapsed.chars() {
        match ch {
            ' ' | '\t' => {
                if !in_space && !out.is_empty() {
                    out.push(' ');
                }
                in_space = true;
            }
            '\n' => {
                in_space = false;
                out.push('\n');
            }
            _ => {
                in_space = false;
                out.push(ch);
            }
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(_) => None,
        Value::Number(n) => n.as_i64(),
        Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                s.parse().ok()
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn text_of(entity: &Map<String, Value>, key: &str) -> Option<String> {
    text(entity.get(key).unwrap_or(&Value::Null))
}

pub fn first_text_of(entity: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = text_of(entity, key) {
            return Some(value);
        }
    }
    None
}

pub fn first_text(payload: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = text(payload.get(key).unwrap_or(&Value::Null)) {
            return Some(value);
        }
    }
    None
}

pub fn urn_id(urn: &str) -> Option<&str> {
    let rest = urn.strip_prefix("urn:li:")?;
    let colon = rest.find(':')?;
    let id_start = colon + 1;
    let mut id = &rest[id_start..];
    if let Some(stripped) = id.strip_prefix('(') {
        id = stripped;
    }
    let end = id.find([',', ')']).unwrap_or(id.len());
    let id = id[..end].trim();
    if id.is_empty() { None } else { Some(id) }
}

pub fn normalise_urn(urn: &Value, entity: &str) -> Option<String> {
    let urn = urn.as_str()?;
    let id = urn_id(urn)?;
    Some(format!("urn:li:{entity}:{id}"))
}

// ----------------------------------------------------------------------- dates

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn parse_date(value: &Value) -> Option<DateParts> {
    let obj = value.as_object()?;
    let year = integer(obj.get("year").unwrap_or(&Value::Null));
    let month = integer(obj.get("month").unwrap_or(&Value::Null));
    let day = integer(obj.get("day").unwrap_or(&Value::Null));
    if year.is_none() && month.is_none() && day.is_none() {
        return None;
    }
    Some(DateParts {
        year,
        month,
        day,
        iso: iso(year, month, day),
        label: date_label(year, month, day),
    })
}

fn iso(year: Option<i64>, month: Option<i64>, day: Option<i64>) -> Option<String> {
    let year = year?;
    Some(format!(
        "{year:04}-{:02}-{:02}",
        month.unwrap_or(1),
        day.unwrap_or(1),
    ))
}

fn date_label(year: Option<i64>, month: Option<i64>, day: Option<i64>) -> Option<String> {
    let named_month = month
        .filter(|m| (1..=12).contains(m))
        .and_then(|m| MONTHS.get((m - 1) as usize))
        .copied();
    match (year, named_month, day) {
        (Some(y), Some(m), _) => Some(format!("{m} {y}")),
        (Some(y), None, _) => Some(y.to_string()),
        (None, Some(m), Some(d)) => Some(format!("{d} {m}")),
        (None, Some(m), None) => Some(m.to_string()),
        _ => None,
    }
}

/// `now` is `(year, month)`, injected so tests never depend on the real date.
pub fn parse_time_period(
    value: &Value,
    open_ended_is_current: bool,
    now: (i64, i64),
) -> Option<DateRange> {
    let obj = value.as_object()?;
    let start = parse_date(obj.get("startDate").unwrap_or(&Value::Null));
    let end = parse_date(obj.get("endDate").unwrap_or(&Value::Null));
    if start.is_none() && end.is_none() {
        return None;
    }
    let is_current = open_ended_is_current && end.is_none() && start.is_some();
    let duration = if end.is_some() || is_current {
        duration_months(start.as_ref(), end.as_ref(), now)
    } else {
        None
    };
    let label = if !is_current && end.is_none() && duration.is_none() {
        start.as_ref().and_then(|s| s.label.clone())
    } else {
        range_label(start.as_ref(), end.as_ref(), is_current, duration)
    };
    Some(DateRange {
        start,
        end,
        is_current,
        duration_months: duration,
        label,
    })
}

fn duration_months(
    start: Option<&DateParts>,
    end: Option<&DateParts>,
    now: (i64, i64),
) -> Option<i64> {
    let start = start?;
    let start_year = start.year?;
    let start_month = start.month?;
    let (end_year, end_month) = match end {
        Some(end) => (end.year?, end.month?),
        None => now,
    };
    let months = (end_year - start_year) * 12 + (end_month - start_month) + 1;
    if months <= 0 { None } else { Some(months) }
}

pub fn humanise_duration(months: Option<i64>) -> Option<String> {
    let months = months?;
    if months <= 0 {
        return None;
    }
    let years = months / 12;
    let remainder = months % 12;
    let mut parts = Vec::new();
    if years > 0 {
        parts.push(if years == 1 {
            "1 yr".to_string()
        } else {
            format!("{years} yrs")
        });
    }
    if remainder > 0 {
        parts.push(if remainder == 1 {
            "1 mo".to_string()
        } else {
            format!("{remainder} mos")
        });
    }
    Some(parts.join(" "))
}

fn range_label(
    start: Option<&DateParts>,
    end: Option<&DateParts>,
    is_current: bool,
    duration: Option<i64>,
) -> Option<String> {
    let left = start.and_then(|s| s.label.clone());
    let right = if is_current {
        Some("Present".to_string())
    } else {
        end.and_then(|e| e.label.clone())
    };
    if left.is_none() && right.is_none() {
        return None;
    }
    let span = [left, right]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" - ");
    match humanise_duration(duration) {
        Some(pretty) => Some(format!("{span} ({pretty})")),
        None => Some(span),
    }
}

// ---------------------------------------------------------------------- images

pub const VECTOR_IMAGE_KEY: &str = "com.linkedin.common.VectorImage";

pub fn epoch_millis_to_datetime(value: &Value) -> Option<DateTime<Utc>> {
    let millis = integer(value)?;
    DateTime::<Utc>::from_timestamp_millis(millis)
}

pub fn parse_vector_image(value: &Value) -> Option<ImageSet> {
    let container = match value.as_object()? {
        o if o.contains_key(VECTOR_IMAGE_KEY) => o.get(VECTOR_IMAGE_KEY)?.as_object()?,
        o => o,
    };
    let root = container.get("rootUrl")?.as_str()?;
    let artifacts = container.get("artifacts")?.as_array()?;

    let mut renditions: Vec<ImageRendition> = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let Some(obj) = artifact.as_object() else {
            continue;
        };
        let Some(segment) = obj
            .get("fileIdentifyingUrlPathSegment")
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        renditions.push(ImageRendition {
            url: format!("{root}{segment}"),
            width: integer(obj.get("width").unwrap_or(&Value::Null)),
            height: integer(obj.get("height").unwrap_or(&Value::Null)),
            expires_at: obj.get("expiresAt").and_then(epoch_millis_to_datetime),
        });
    }

    if renditions.is_empty() {
        return None;
    }
    // Largest first by width * height (u64); stable sort keeps equal areas in
    // upstream order, mirroring Python's `reverse=True` sort.
    renditions.sort_by_key(|r| {
        let area = (r.width.unwrap_or(0) as u64) * (r.height.unwrap_or(0) as u64);
        std::cmp::Reverse(area)
    });
    Some(ImageSet {
        url: renditions[0].url.clone(),
        renditions,
    })
}

// --------------------------------------------------------------- organisations

pub fn parse_company(payload: &Value) -> Option<Organization> {
    let obj = payload.as_object()?;
    let mini = obj.get("miniCompany").and_then(Value::as_object);

    let name = mini
        .and_then(|m| first_text_of(m, &["name", "companyName"]))
        .or_else(|| first_text_of(obj, &["companyName", "name"]));
    let universal_name = mini.and_then(|m| text_of(m, "universalName"));
    let urn_value = mini
        .and_then(|m| m.get("entityUrn"))
        .or_else(|| obj.get("companyUrn"))
        .and_then(|v| normalise_urn(v, "organization"));
    let logo = mini
        .and_then(|m| m.get("logo"))
        .or_else(|| obj.get("logo"))
        .and_then(parse_vector_image);

    if name.is_none() && urn_value.is_none() && logo.is_none() {
        return None;
    }
    Some(Organization {
        linkedin_url: universal_name
            .as_ref()
            .map(|u| format!("https://www.linkedin.com/company/{u}")),
        name,
        urn: urn_value,
        universal_name,
        logo,
    })
}

pub fn parse_school(payload: &Value, fallback_name: Option<String>) -> Option<Organization> {
    let obj = payload.as_object();
    let name = obj
        .and_then(|o| first_text_of(o, &["schoolName", "name"]))
        .or(fallback_name);
    let urn_value = obj
        .and_then(|o| o.get("entityUrn").or_else(|| o.get("objectUrn")))
        .and_then(|v| normalise_urn(v, "school"));
    let logo = obj.and_then(|o| o.get("logo")).and_then(parse_vector_image);

    if name.is_none() && urn_value.is_none() && logo.is_none() {
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

/// Return `payload[key]["elements"]` as a list of objects, whatever the shape.
pub fn elements<'a>(payload: &'a Map<String, Value>, key: &str) -> Vec<&'a Map<String, Value>> {
    let view = match payload.get(key) {
        Some(Value::Object(o)) => o.get("elements").unwrap_or(&Value::Null),
        Some(v) => v,
        None => return Vec::new(),
    };
    match view.as_array() {
        Some(items) => items.iter().filter_map(|item| item.as_object()).collect(),
        None => Vec::new(),
    }
}

pub fn enum_label(value: &Value) -> Option<String> {
    let raw = text(value)?;
    let upper = raw.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
    if upper || raw.contains('_') {
        let lowered = raw.replace('_', " ").to_lowercase();
        let words: Vec<&str> = lowered.split(' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return None;
        }
        let mut first = words[0].chars();
        let capitalized: String = first
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_default()
            .chars()
            .chain(first)
            .collect();
        Some(
            [capitalized.as_str(), &words[1..].join(" ")]
                .into_iter()
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        )
    } else {
        Some(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_normalises_whitespace_keeps_newlines() {
        assert_eq!(text(&json!("  hello   world ")), Some("hello world".into()));
        assert_eq!(text(&json!("a\r\nb\t  c")), Some("a\nb c".into()));
        assert_eq!(text(&json!("   ")), None);
        assert_eq!(text(&json!(42)), None);
        assert_eq!(text(&json!(null)), None);
        assert_eq!(text(&json!("a \n b")), Some("a \n b".into()));
    }

    #[test]
    fn integer_rejects_booleans() {
        assert_eq!(integer(&json!(5)), Some(5));
        assert_eq!(integer(&json!(true)), None);
        assert_eq!(integer(&json!(false)), None);
        assert_eq!(integer(&json!("12")), Some(12));
        assert_eq!(integer(&json!("12x")), None);
        assert_eq!(integer(&json!(1.5)), None);
        assert_eq!(integer(&json!(4.0)), None);
        assert_eq!(integer(&json!(null)), None);
    }

    #[test]
    fn urn_parsing() {
        assert_eq!(urn_id("urn:li:fs_miniProfile:ACoAAB1c"), Some("ACoAAB1c"));
        assert_eq!(
            urn_id("urn:li:fs_position:(ACoAAB1c,1234)"),
            Some("ACoAAB1c")
        );
        assert_eq!(urn_id("bucket"), None);
        assert_eq!(
            normalise_urn(&json!("urn:li:fsd_profilePosition:(x,9)"), "position"),
            Some("urn:li:position:x".into())
        );
        assert_eq!(normalise_urn(&json!(null), "member"), None);
    }

    #[test]
    fn date_parsing_and_labels() {
        let full = parse_date(&json!({"year": 2021, "month": 3, "day": 4})).unwrap();
        assert_eq!(full.iso.as_deref(), Some("2021-03-04"));
        assert_eq!(full.label.as_deref(), Some("Mar 2021"));
        let partial = parse_date(&json!({"year": 2020})).unwrap();
        assert_eq!(partial.iso.as_deref(), Some("2020-01-01"));
        assert_eq!(partial.label.as_deref(), Some("2020"));
        let birthday = parse_date(&json!({"month": 9, "day": 19})).unwrap();
        assert_eq!(birthday.label.as_deref(), Some("19 Sep"));
        assert!(parse_date(&json!({"janitor": 1})).is_none());
        assert!(parse_date(&json!({})).is_none());
        assert!(parse_date(&json!(null)).is_none());
    }

    #[test]
    fn duration_with_injected_now() {
        let range = parse_time_period(
            &json!({"startDate": {"year": 2021, "month": 3}, "endDate": {"year": 2022, "month": 8}}),
            true,
            (2026, 8),
        )
        .unwrap();
        assert_eq!(range.duration_months, Some(18));
        assert_eq!(
            range.label.as_deref(),
            Some("Mar 2021 - Aug 2022 (1 yr 6 mos)")
        );

        let current = parse_time_period(
            &json!({"startDate": {"year": 2024, "month": 1}}),
            true,
            (2026, 8),
        )
        .unwrap();
        assert!(current.is_current);
        assert_eq!(current.duration_months, Some(32));
        assert_eq!(
            current.label.as_deref(),
            Some("Jan 2024 - Present (2 yrs 8 mos)")
        );

        let certified = parse_time_period(
            &json!({"startDate": {"year": 2019, "month": 2}}),
            false,
            (2026, 8),
        )
        .unwrap();
        assert!(!certified.is_current);
        assert_eq!(certified.duration_months, None);
        assert_eq!(certified.label.as_deref(), Some("Feb 2019"));

        let same_month = parse_time_period(
            &json!({"startDate": {"year": 2020, "month": 5}, "endDate": {"year": 2020, "month": 5}}),
            true,
            (2026, 8),
        )
        .unwrap();
        assert_eq!(same_month.duration_months, Some(1));

        let backwards = parse_time_period(
            &json!({"startDate": {"year": 2020, "month": 9}, "endDate": {"year": 2020, "month": 1}}),
            true,
            (2026, 8),
        )
        .unwrap();
        assert_eq!(backwards.duration_months, None);

        let year_only = parse_time_period(
            &json!({"startDate": {"year": 2018}, "endDate": {"year": 2019}}),
            true,
            (2026, 8),
        )
        .unwrap();
        assert_eq!(year_only.duration_months, None);
    }

    #[test]
    fn vector_images() {
        let value = json!({
            "rootUrl": "https://media.licdn.com/",
            "artifacts": [
                {"fileIdentifyingUrlPathSegment": "a/100.jpg", "width": 100, "height": 100, "expiresAt": 1767225600000i64},
                {"fileIdentifyingUrlPathSegment": "a/800.jpg", "width": 800, "height": 800},
            ],
        });
        let image = parse_vector_image(&value).unwrap();
        assert_eq!(image.url, "https://media.licdn.com/a/800.jpg");
        assert_eq!(image.renditions.len(), 2);
        assert_eq!(image.renditions[0].width, Some(800), "largest first");
        assert!(
            image.renditions[1].expires_at.is_some(),
            "rendition order preserved"
        );
        assert!(parse_vector_image(&json!(null)).is_none());
        assert!(parse_vector_image(&json!({"a": 1})).is_none());
    }

    #[test]
    fn epoch_overflow_returns_none() {
        assert!(epoch_millis_to_datetime(&json!(i64::MAX)).is_none());
        assert!(epoch_millis_to_datetime(&json!(1767225600000i64)).is_some());
        assert!(epoch_millis_to_datetime(&json!("nope")).is_none());
    }

    #[test]
    fn company_and_school() {
        let company = parse_company(&json!({
            "miniCompany": {
                "name": "Acme",
                "universalName": "acme",
                "entityUrn": "urn:li:fsd_company:123",
                "logo": {"rootUrl": "https://x/", "artifacts": [{"fileIdentifyingUrlPathSegment": "l.jpg"}]},
            }
        }))
        .unwrap();
        assert_eq!(company.name.as_deref(), Some("Acme"));
        assert_eq!(company.urn.as_deref(), Some("urn:li:organization:123"));
        assert_eq!(
            company.linkedin_url.as_deref(),
            Some("https://www.linkedin.com/company/acme")
        );
        assert!(company.logo.is_some());

        let school = parse_school(
            &json!({"schoolName": "MIT", "entityUrn": "urn:li:fsd_school:7"}),
            None,
        )
        .unwrap();
        assert_eq!(school.name.as_deref(), Some("MIT"));
        assert_eq!(school.urn.as_deref(), Some("urn:li:school:7"));
    }

    #[test]
    fn enum_labels() {
        assert_eq!(
            enum_label(&json!("NATIVE_OR_BILINGUAL")),
            Some("Native or bilingual".into())
        );
        assert_eq!(enum_label(&json!("HALF_OPEN")), Some("Half open".into()));
        assert_eq!(
            enum_label(&json!("Senior Engineer")),
            Some("Senior Engineer".into())
        );
        assert_eq!(enum_label(&json!(null)), None);
    }

    #[test]
    fn elements_collection() {
        let payload =
            json!({"skillView": {"elements": [{"name": "Rust"}, 42, null, {"name": "Go"}]}});
        let items = elements(payload.as_object().unwrap(), "skillView");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("name").and_then(Value::as_str), Some("Rust"));
        assert_eq!(
            elements(payload.as_object().unwrap(), "missing"),
            Vec::<&Map<String, Value>>::new()
        );
    }
}

#[cfg(test)]
mod dbg {
    use super::*;
    use serde_json::json;
    #[test]
    fn dbg_date() {
        let v = json!({"year": 2020});
        let obj = v.as_object();
        println!("obj is none: {}", obj.is_none());
        let year = obj.and_then(|o| o.get("year"));
        println!("year: {year:?}");
        let yr = year.and_then(integer);
        println!("yr: {yr:?}");
        let partial = parse_date(&v);
        println!("partial: {partial:?}");
        let birthday = parse_date(&json!({"month": 9, "day": 19}));
        println!("birthday: {birthday:?}");
    }
}
