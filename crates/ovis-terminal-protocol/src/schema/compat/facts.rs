//! Alternatives of a schema and the facts each one states.
//!
//! A schema is flattened into the alternatives a value may match. A value must
//! satisfy the node's own keywords, every `allOf` member, one `anyOf` member and
//! one `oneOf` member, so each alternative is one combination of those parts;
//! a type array splits into one alternative per type. Compositions nest to any
//! depth. References are resolved along a [`Trail`] that rejects cycles and
//! excessive depth, and the number of alternatives is bounded.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::shape::Trail;
use crate::schema::document::COMPONENTS_PREFIX;
use crate::schema::error::SchemaError;
use crate::schema::scalar::OPEN_ENUM_EXTENSION;

/// Most alternatives one schema may expand to before failing with a typed error.
pub(super) const MAX_ALTERNATIVES: usize = 256;

/// One alternative: its conjunct schema parts, the single types its type arrays
/// were split into, and its trail.
#[derive(Debug, Clone)]
pub(super) struct Alt<'a> {
    parts: Vec<&'a Value>,
    only: Vec<&'a str>,
    open: bool,
    pub(super) trail: Trail,
}

impl Alt<'_> {
    /// The alternative a value matches when it matches both `self` and `other`.
    fn and(&self, other: &Self) -> Self {
        let refs = self.trail.refs.iter();
        let shared = refs.filter(|entered| other.trail.refs.contains(entered));
        Self {
            parts: self.parts.iter().chain(&other.parts).copied().collect(),
            only: self.only.iter().chain(&other.only).copied().collect(),
            open: self.open || other.open,
            trail: Trail {
                refs: shared.cloned().collect(),
                depth: self.trail.depth.max(other.trail.depth),
            },
        }
    }
}

/// Every pairing of one alternative of `left` with one of `right`.
fn conjoin<'a>(
    left: &[Alt<'a>],
    right: &[Alt<'a>],
    location: &str,
) -> Result<Vec<Alt<'a>>, SchemaError> {
    if left.len().saturating_mul(right.len()) > MAX_ALTERNATIVES {
        return Err(SchemaError::TooManyAlternatives {
            location: location.to_owned(),
            limit: MAX_ALTERNATIVES,
        });
    }
    Ok(left
        .iter()
        .flat_map(|l| right.iter().map(|r| l.and(r)))
        .collect())
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

fn type_names(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn members<'a>(node: &'a Value, keyword: &str) -> &'a [Value] {
    node.get(keyword)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
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
    let own = |only| Alt {
        parts: vec![node],
        only,
        open,
        trail: trail.clone(),
    };
    let types = type_names(node);
    let mut found: Vec<Alt<'a>> = if types.len() > 1 {
        types.into_iter().map(|only| own(vec![only])).collect()
    } else {
        vec![own(Vec::new())]
    };
    let location = trail.refs.last().map_or("schema root", String::as_str);
    for part in members(node, "allOf") {
        found = conjoin(&found, &alternatives(document, part, &trail)?, location)?;
    }
    for keyword in ["anyOf", "oneOf"] {
        let mut either = Vec::new();
        for member in members(node, keyword) {
            either.extend(alternatives(document, member, &trail)?);
        }
        if !either.is_empty() {
            found = conjoin(&found, &either, location)?;
        }
    }
    Ok(found)
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

fn intersect_types(types: Option<BTreeSet<String>>, part: Vec<&str>) -> Option<BTreeSet<String>> {
    if part.is_empty() {
        return types;
    }
    let part: BTreeSet<String> = part.into_iter().map(str::to_owned).collect();
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
    for only in &alt.only {
        found.types = intersect_types(found.types.take(), vec![only]);
    }
    found
}
