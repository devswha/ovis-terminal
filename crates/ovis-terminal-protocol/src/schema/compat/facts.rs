//! Alternatives of a schema and the facts each one states.
//!
//! A schema is flattened into the alternatives a value may match: `anyOf` and
//! `oneOf` members (including nullable wrappers), one alternative per entry of a
//! type array, and `allOf` parts merged into one conjunction. References are
//! resolved along a [`Trail`] that rejects cycles and excessive depth.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::shape::Trail;
use crate::schema::document::COMPONENTS_PREFIX;
use crate::schema::error::SchemaError;
use crate::schema::scalar::OPEN_ENUM_EXTENSION;

/// One alternative: its conjunct schema parts, an optional single type, and its trail.
#[derive(Debug, Clone)]
pub(super) struct Alt<'a> {
    parts: Vec<&'a Value>,
    only: Option<String>,
    open: bool,
    pub(super) trail: Trail,
}

fn is_open(schema: &Value) -> bool {
    schema.get(OPEN_ENUM_EXTENSION).and_then(Value::as_bool) == Some(true)
}

/// Follows `$ref` chains, refusing a reference already entered on this path.
fn resolve<'a>(
    document: &'a Value,
    schema: &'a Value,
    trail: &mut Trail,
) -> Result<&'a Value, SchemaError> {
    let mut current = schema;
    while let Some(reference) = current.get("$ref").and_then(Value::as_str) {
        if trail.refs.iter().any(|entered| entered == reference) {
            return Err(SchemaError::ReferenceCycle {
                reference: reference.to_owned(),
            });
        }
        trail.deeper(reference)?;
        trail.refs.push(reference.to_owned());
        let target = reference
            .strip_prefix(COMPONENTS_PREFIX)
            .and_then(|name| document.pointer("/components/schemas")?.get(name));
        current = target.ok_or_else(|| SchemaError::UnresolvedReference {
            reference: reference.to_owned(),
        })?;
    }
    Ok(current)
}

fn type_names(schema: &Value) -> Vec<String> {
    match schema.get("type") {
        Some(Value::String(name)) => vec![name.clone()],
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// Flattens `schema` into the alternatives a value may match.
///
/// # Errors
///
/// Returns [`SchemaError::ReferenceCycle`], [`SchemaError::DepthExceeded`] or
/// [`SchemaError::UnresolvedReference`].
pub(super) fn alternatives<'a>(
    document: &'a Value,
    schema: &'a Value,
    trail: &Trail,
) -> Result<Vec<Alt<'a>>, SchemaError> {
    let mut trail = trail.clone();
    trail.deeper("schema nesting")?;
    let open = is_open(schema);
    let node = resolve(document, schema, &mut trail)?;
    let open = open || is_open(node);
    let members = ["anyOf", "oneOf"]
        .into_iter()
        .filter_map(|key| node.get(key)?.as_array());
    let members: Vec<&Value> = members.flatten().collect();
    if !members.is_empty() {
        let mut found = Vec::new();
        for member in members {
            found.extend(
                alternatives(document, member, &trail)?
                    .into_iter()
                    .map(|alt| Alt {
                        open: alt.open || open,
                        ..alt
                    }),
            );
        }
        return Ok(found);
    }
    let mut parts = vec![node];
    for part in node
        .get("allOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        parts.push(resolve(document, part, &mut trail.clone())?);
    }
    let types = type_names(node);
    if types.len() > 1 {
        return Ok(types
            .into_iter()
            .map(|only| Alt {
                parts: parts.clone(),
                only: Some(only),
                open,
                trail: trail.clone(),
            })
            .collect());
    }
    Ok(vec![Alt {
        parts,
        only: None,
        open,
        trail,
    }])
}

/// Facts one alternative states, with `allOf` parts merged as an intersection.
#[derive(Debug, Clone, Default)]
pub(super) struct Facts<'a> {
    pub(super) types: Option<BTreeSet<String>>,
    pub(super) literals: Option<Vec<Value>>,
    pub(super) uppers: BTreeMap<&'static str, f64>,
    pub(super) lowers: BTreeMap<&'static str, f64>,
    pub(super) patterns: BTreeSet<String>,
    pub(super) properties: BTreeMap<&'a str, &'a Value>,
    pub(super) pattern_properties: BTreeMap<&'a str, &'a Value>,
    pub(super) required: BTreeSet<&'a str>,
    pub(super) closed: bool,
    pub(super) additional: Option<&'a Value>,
    pub(super) items: Option<&'a Value>,
    pub(super) open: bool,
}

/// Keywords whose larger value accepts more input.
pub(super) const UPPER_BOUNDS: [&str; 5] = [
    "maximum",
    "exclusiveMaximum",
    "maxLength",
    "maxItems",
    "maxProperties",
];
/// Keywords whose smaller value accepts more input.
pub(super) const LOWER_BOUNDS: [&str; 5] = [
    "minimum",
    "exclusiveMinimum",
    "minLength",
    "minItems",
    "minProperties",
];

fn intersect_types(types: Option<BTreeSet<String>>, part: Vec<String>) -> Option<BTreeSet<String>> {
    if part.is_empty() {
        return types;
    }
    let part: BTreeSet<String> = part.into_iter().collect();
    Some(types.map_or_else(
        || part.clone(),
        |known| known.intersection(&part).cloned().collect(),
    ))
}

fn merge_part<'a>(facts: &mut Facts<'a>, part: &'a Value) {
    facts.types = intersect_types(facts.types.take(), type_names(part));
    let enumerated = part
        .get("enum")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let literals: Vec<Value> = enumerated.chain(part.get("const")).cloned().collect();
    if part.get("enum").is_some() || part.get("const").is_some() {
        facts.literals = Some(match facts.literals.take() {
            Some(known) => known
                .into_iter()
                .filter(|value| literals.contains(value))
                .collect(),
            None => literals,
        });
    }
    for keyword in UPPER_BOUNDS {
        if let Some(bound) = part.get(keyword).and_then(Value::as_f64) {
            facts
                .uppers
                .entry(keyword)
                .and_modify(|known| *known = known.min(bound))
                .or_insert(bound);
        }
    }
    for keyword in LOWER_BOUNDS {
        if let Some(bound) = part.get(keyword).and_then(Value::as_f64) {
            facts
                .lowers
                .entry(keyword)
                .and_modify(|known| *known = known.max(bound))
                .or_insert(bound);
        }
    }
    facts.patterns.extend(
        part.get("pattern")
            .and_then(Value::as_str)
            .map(str::to_owned),
    );
    let entries = |key: &str| {
        part.get(key)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
    };
    for (name, schema) in entries("properties") {
        facts.properties.entry(name.as_str()).or_insert(schema);
    }
    for (pattern, schema) in entries("patternProperties") {
        facts
            .pattern_properties
            .entry(pattern.as_str())
            .or_insert(schema);
    }
    facts.required.extend(
        part.get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str),
    );
    match part.get("additionalProperties") {
        Some(Value::Bool(false)) => facts.closed = true,
        Some(schema @ Value::Object(_)) => facts.additional = facts.additional.or(Some(schema)),
        _ => {}
    }
    facts.items = facts.items.or_else(|| part.get("items"));
}

/// Merges the parts of one alternative into its facts.
pub(super) fn facts<'a>(alt: &Alt<'a>) -> Facts<'a> {
    let mut found = Facts {
        open: alt.open,
        ..Facts::default()
    };
    for part in &alt.parts {
        merge_part(&mut found, part);
    }
    if let Some(only) = &alt.only {
        found.types = intersect_types(found.types.take(), vec![only.clone()]);
    }
    found
}
