//! Validated access to the named entries of a published document.

use serde_json::Value;

use crate::schema::error::SchemaError;

/// Keys an OpenRPC method object may carry.
const METHOD_KEYS: &[&str] = &[
    "name",
    "summary",
    "description",
    "tags",
    "paramStructure",
    "params",
    "result",
    "errors",
    "deprecated",
    "externalDocs",
    "servers",
    "links",
    "examples",
    "x-params",
    "x-effect",
    "x-targeting",
];
/// Keys an OpenRPC content descriptor may carry.
const PARAM_KEYS: &[&str] = &[
    "name",
    "summary",
    "description",
    "required",
    "schema",
    "deprecated",
];
/// Keys an event or notification entry may carry.
const SIGNAL_KEYS: &[&str] = &["name", "summary", "description", "schema"];

/// A named section of a published document and the keys its entries may carry.
#[derive(Debug, Clone, Copy)]
pub(super) enum Section {
    /// `methods`.
    Methods,
    /// `x-events`.
    Events,
    /// `x-notifications`.
    Notifications,
}

impl Section {
    const fn key(self) -> &'static str {
        match self {
            Self::Methods => "methods",
            Self::Events => "x-events",
            Self::Notifications => "x-notifications",
        }
    }

    const fn allowed(self) -> &'static [&'static str] {
        match self {
            Self::Methods => METHOD_KEYS,
            Self::Events | Self::Notifications => SIGNAL_KEYS,
        }
    }
}

/// Checks that `entry` has a string name and only `allowed` keys.
fn named_entry<'a>(
    entry: &'a Value,
    location: &str,
    allowed: &[&str],
) -> Result<(&'a str, &'a Value), SchemaError> {
    let object = entry
        .as_object()
        .ok_or_else(|| SchemaError::MalformedBaseline {
            location: location.to_owned(),
        })?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        let location = format!("{location}.{unknown}");
        return Err(SchemaError::MalformedBaseline { location });
    }
    let name = object.get("name").and_then(Value::as_str);
    name.map(|name| (name, entry))
        .ok_or_else(|| SchemaError::MalformedBaseline {
            location: format!("{location}.name"),
        })
}

/// Every entry of `section`, each with its name.
///
/// # Errors
///
/// Returns [`SchemaError::MalformedBaseline`] when the section is missing or an
/// entry lacks a name or carries a key the section does not define.
pub(super) fn entries(
    document: &Value,
    section: Section,
) -> Result<Vec<(&str, &Value)>, SchemaError> {
    let list = document
        .get(section.key())
        .and_then(Value::as_array)
        .ok_or_else(|| SchemaError::MalformedBaseline {
            location: section.key().to_owned(),
        })?;
    list.iter()
        .enumerate()
        .map(|(index, entry)| {
            let location = format!("{}[{index}]", section.key());
            named_entry(entry, &location, section.allowed())
        })
        .collect()
}

/// By-name parameters of one method object.
///
/// # Errors
///
/// Returns [`SchemaError::MalformedBaseline`] for missing params or a malformed descriptor.
pub(super) fn params<'a>(
    method: &'a Value,
    name: &str,
) -> Result<Vec<(&'a str, &'a Value)>, SchemaError> {
    let location = format!("methods[{name}].params");
    let list = method
        .get("params")
        .and_then(Value::as_array)
        .ok_or_else(|| SchemaError::MalformedBaseline {
            location: location.clone(),
        })?;
    list.iter()
        .enumerate()
        .map(|(index, param)| named_entry(param, &format!("{location}[{index}]"), PARAM_KEYS))
        .collect()
}

/// The member at `pointer` of an entry, or a typed failure naming where it is missing.
pub(super) fn member<'a>(
    entry: &'a Value,
    pointer: &str,
    owner: &str,
) -> Result<&'a Value, SchemaError> {
    entry
        .pointer(pointer)
        .ok_or_else(|| SchemaError::MalformedBaseline {
            location: format!("{owner}{pointer}"),
        })
}
