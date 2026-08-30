//! Resolver for LinkedIn's normalised JSON envelope
//! (`application/vnd.linkedin.normalized+json+2.1`):
//!
//! ```json
//! {
//!   "data": { "*elements": ["urn:li:fsd_profile:ABC"], ... },
//!   "included": [ { "entityUrn": "urn:li:fsd_profile:ABC", "$type": "...profile.Profile", ... } ]
//! }
//! ```
//!
//! The index maps URNs to entities in `included` without building a second
//! copy of the graph. It borrows from the incoming `Value`.

use std::collections::HashMap;

use serde_json::{Map, Value};

const TYPE_KEY: &str = "$type";

pub struct EntityIndex<'a> {
    included: HashMap<&'a str, &'a Value>,
    order: Vec<&'a str>,
    data: Option<&'a Map<String, Value>>,
}

impl<'a> EntityIndex<'a> {
    pub fn new(payload: &'a Value) -> Option<Self> {
        let raw = payload.as_object()?;
        let data = raw.get("data").and_then(Value::as_object);
        let mut included = HashMap::new();
        let mut order = Vec::new();
        let mut has_included = false;
        if let Some(Value::Array(items)) = raw.get("included") {
            has_included = true;
            for item in items {
                if let Some(urn) = item
                    .get("entityUrn")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                {
                    // First occurrence wins; `order` keeps `of_type` stable in
                    // upstream order so section output is deterministic.
                    if included.insert(urn, item).is_none() {
                        order.push(urn);
                    }
                }
            }
        }
        if has_included || data.is_some() {
            Some(EntityIndex {
                included,
                order,
                data,
            })
        } else {
            None
        }
    }

    pub fn data(&self) -> Option<&'a Map<String, Value>> {
        self.data
    }

    pub fn by_urn(&self, urn: Option<&str>) -> Option<&'a Value> {
        urn.and_then(|u| self.included.get(u).copied())
    }

    /// Entities whose `$type` ends with any of `suffixes`, in `included` order.
    /// Matching the suffix rather than the full type name keeps this working
    /// when LinkedIn renames a package.
    pub fn of_type(&self, suffixes: &[&str]) -> Vec<&'a Map<String, Value>> {
        let wanted: Vec<String> = suffixes.iter().map(|s| format!(".{s}")).collect();
        self.order
            .iter()
            .filter_map(|urn| self.included.get(urn).copied())
            .filter(|entity| {
                entity
                    .get(TYPE_KEY)
                    .and_then(Value::as_str)
                    .is_some_and(|t| wanted.iter().any(|w| t.ends_with(w.as_str())))
            })
            .filter_map(|entity| entity.as_object())
            .collect()
    }

    pub fn first_of_type(&self, suffixes: &[&str]) -> Option<&'a Map<String, Value>> {
        self.of_type(suffixes).into_iter().next()
    }

    /// Read `name` from `entity`, dereferencing a `*name` pointer if present.
    /// A URN is replaced by its entity; lists keep their items (strings or
    /// objects) so callers can resolve items with `item_at`.
    pub fn field<'e>(&self, entity: Option<&'e Map<String, Value>>, name: &str) -> Option<&'e Value>
    where
        'a: 'e,
    {
        let entity = entity?;
        if let Some(value) = entity.get(name) {
            return Some(self.unwrap_urn(value));
        }
        let pointer = entity.get(&format!("*{name}"))?;
        Some(self.unwrap_urn(pointer))
    }

    fn unwrap_urn<'e>(&self, value: &'e Value) -> &'e Value
    where
        'a: 'e,
    {
        match value {
            Value::String(s) => self.by_urn(Some(s)).unwrap_or(value),
            _ => value,
        }
    }

    /// Resolve an item that may be a URN string or an embedded object.
    pub fn item<'e>(&self, value: &'e Value) -> Option<&'e Map<String, Value>>
    where
        'a: 'e,
    {
        match value {
            Value::Object(o) => Some(o),
            Value::String(u) => self.by_urn(Some(u)).and_then(Value::as_object),
            _ => None,
        }
    }

    /// The resolved `elements` collection of a paged entity.
    pub fn elements<'e>(
        &self,
        entity: Option<&'e Map<String, Value>>,
    ) -> Vec<&'e Map<String, Value>>
    where
        'a: 'e,
    {
        self.elements_named(entity, "elements")
    }

    /// Like `elements`, but for a named collection whose items may be objects
    /// or URN strings (e.g. `profilePositionInPositionGroup`).
    pub fn elements_named<'e>(
        &self,
        entity: Option<&'e Map<String, Value>>,
        name: &str,
    ) -> Vec<&'e Map<String, Value>>
    where
        'a: 'e,
    {
        let Some(resolved) = self.field(entity, name) else {
            return Vec::new();
        };
        let items = match resolved {
            Value::Array(items) => items,
            Value::Object(o) => {
                let inner = o.get("elements").map(|v| self.unwrap_urn(v));
                match inner {
                    Some(Value::Array(items)) => items,
                    _ => return Vec::new(),
                }
            }
            _ => return Vec::new(),
        };
        items.iter().filter_map(|item| self.item(item)).collect()
    }
}

/// Depth-first search for the first `rootUrl` + `artifacts` pair, bounded by
/// depth and list count. Dash wraps images differently per section, so
/// searching for the shape is more durable than enumerating the wrappers.
pub fn find_vector_image(node: &Value, depth: usize) -> Option<&Map<String, Value>> {
    const MAX_DEPTH: usize = 6;
    const MAX_LIST_ITEMS: usize = 12;
    if depth > MAX_DEPTH {
        return None;
    }
    match node {
        Value::Object(obj) => {
            if obj.get("rootUrl").is_some_and(Value::is_string)
                && obj.get("artifacts").is_some_and(Value::is_array)
            {
                return Some(obj);
            }
            for value in obj.values() {
                if let Some(found) = find_vector_image(value, depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items.iter().take(MAX_LIST_ITEMS) {
                if let Some(found) = find_vector_image(item, depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}
