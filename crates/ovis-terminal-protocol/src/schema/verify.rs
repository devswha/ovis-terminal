//! Structural checks every published document must pass.

use std::collections::BTreeSet;

use serde_json::Value;

use super::document::COMPONENTS_PREFIX;
use super::error::SchemaError;

/// Names in one published section, rejecting the first duplicate.
fn unique_names<'a>(
    document: &'a Value,
    section: &'static str,
) -> Result<BTreeSet<&'a str>, SchemaError> {
    let entries = document
        .get(section)
        .and_then(Value::as_array)
        .ok_or(SchemaError::Malformed { section })?;
    let mut names = BTreeSet::new();
    for name in entries
        .iter()
        .map(|entry| entry.get("name").and_then(Value::as_str))
    {
        let name = name.ok_or(SchemaError::Malformed { section })?;
        if !names.insert(name) {
            return Err(SchemaError::DuplicateName {
                name: name.to_owned(),
            });
        }
    }
    Ok(names)
}

/// Collects every `$ref` string in the document.
fn references<'a>(value: &'a Value, found: &mut Vec<&'a str>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                found.push(reference);
            }
            object.values().for_each(|child| references(child, found));
        }
        Value::Array(items) => items.iter().for_each(|child| references(child, found)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Checks unique method, event and notification names, resolvable references
/// and collision-free definition names.
///
/// # Errors
///
/// Returns [`SchemaError::DuplicateName`], [`SchemaError::UnresolvedReference`],
/// [`SchemaError::NameCollision`] or [`SchemaError::Malformed`].
pub fn verify_document(document: &Value) -> Result<(), SchemaError> {
    for section in ["methods", "x-events", "x-notifications"] {
        unique_names(document, section)?;
    }
    let schemas = document
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .ok_or(SchemaError::Malformed {
            section: "components.schemas",
        })?;
    if let Some(name) = schemas
        .keys()
        .find(|name| name.ends_with(|c: char| c.is_ascii_digit()))
    {
        return Err(SchemaError::NameCollision { name: name.clone() });
    }
    let mut found = Vec::new();
    references(document, &mut found);
    for reference in found {
        let resolved = reference
            .strip_prefix(COMPONENTS_PREFIX)
            .is_some_and(|name| schemas.contains_key(name));
        if !resolved {
            return Err(SchemaError::UnresolvedReference {
                reference: reference.to_owned(),
            });
        }
    }
    Ok(())
}
