//! Facts one schema position states: types, values, bounds and object members.

use std::collections::BTreeSet;

use serde_json::Value;

use super::BreakRule;
use super::shape::Position;
use crate::schema::document::COMPONENTS_PREFIX;
use crate::schema::scalar::OPEN_ENUM_EXTENSION;

/// Keywords whose larger value accepts more input.
const UPPER_BOUNDS: [&str; 5] = [
    "maximum",
    "exclusiveMaximum",
    "maxLength",
    "maxItems",
    "maxProperties",
];
/// Keywords whose smaller value accepts more input.
const LOWER_BOUNDS: [&str; 5] = [
    "minimum",
    "exclusiveMinimum",
    "minLength",
    "minItems",
    "minProperties",
];

/// Types, values, openness, bounds and object members of one schema.
#[derive(Default)]
pub(super) struct Shape<'a> {
    types: BTreeSet<String>,
    free_types: BTreeSet<String>,
    values: BTreeSet<String>,
    open: bool,
    node: Option<&'a Value>,
    pub(super) objects: Vec<&'a Value>,
    pub(super) items: Option<&'a Value>,
}

fn is_open(schema: &Value) -> bool {
    schema.get(OPEN_ENUM_EXTENSION).and_then(Value::as_bool) == Some(true)
}

fn resolve<'a>(document: &'a Value, schema: &'a Value) -> &'a Value {
    let name = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix(COMPONENTS_PREFIX));
    let definitions = document.pointer("/components/schemas");
    name.and_then(|name| definitions?.get(name))
        .map_or(schema, |definition| resolve(document, definition))
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

fn collect<'a>(document: &'a Value, schema: &'a Value, found: &mut Shape<'a>) {
    found.open |= is_open(schema);
    let schema = resolve(document, schema);
    found.open |= is_open(schema);
    found.node = found.node.or(Some(schema));
    let types = type_names(schema);
    let enumerated = schema
        .get("enum")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let literals: Vec<&Value> = enumerated.chain(schema.get("const")).collect();
    if literals.is_empty() {
        found.free_types.extend(types.iter().cloned());
    }
    found.types.extend(types);
    found.values.extend(
        literals
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned)),
    );
    if schema.get("properties").is_some() {
        found.objects.push(schema);
    }
    found.items = found.items.or_else(|| schema.get("items"));
    let members = ["anyOf", "oneOf"]
        .into_iter()
        .filter_map(|key| schema.get(key)?.as_array());
    for member in members.flatten() {
        collect(document, member, found);
    }
}

/// Collects the facts of `schema`, resolving references against `document`.
pub(super) fn shape_of<'a>(document: &'a Value, schema: &'a Value) -> Shape<'a> {
    let mut found = Shape::default();
    collect(document, schema, &mut found);
    found
}

fn bound(node: Option<&Value>, keyword: &str) -> Option<f64> {
    node?.get(keyword)?.as_f64()
}

/// Whether any bound or pattern of the new node accepts less than the old node.
fn constraint_breaks(old: Option<&Value>, new: Option<&Value>) -> Vec<BreakRule> {
    let tighter_upper = UPPER_BOUNDS.iter().any(|keyword| {
        bound(new, keyword).is_some_and(|n| bound(old, keyword).is_none_or(|o| n < o))
    });
    let tighter_lower = LOWER_BOUNDS.iter().any(|keyword| {
        bound(new, keyword).is_some_and(|n| bound(old, keyword).is_none_or(|o| n > o))
    });
    let pattern = |node: Option<&Value>| node.and_then(|n| n.get("pattern")).cloned();
    let mut rules = Vec::new();
    if tighter_upper || tighter_lower {
        rules.push(BreakRule::InputBoundNarrowed);
    }
    if pattern(new).is_some() && pattern(new) != pattern(old) {
        rules.push(BreakRule::InputPatternChanged);
    }
    rules
}

/// Rules broken by the types, values and constraints of one schema position.
pub(super) fn scalar_breaks(
    old: &Shape<'_>,
    new: &Shape<'_>,
    position: Position,
) -> Vec<BreakRule> {
    let mut rules = Vec::new();
    match position {
        Position::Input => {
            if !new.types.is_empty() && !old.types.is_subset(&new.types) {
                rules.push(BreakRule::InputTypeNarrowed);
            }
            let restricted = !old.free_types.is_subset(&new.free_types);
            let removed = !new.values.is_empty() && !old.values.is_subset(&new.values);
            if restricted || removed {
                rules.push(BreakRule::InputValuesNarrowed);
            }
            rules.extend(constraint_breaks(old.node, new.node));
        }
        Position::Output => {
            if !old.types.is_empty() && new.types.is_empty() && new.objects.is_empty() {
                rules.push(BreakRule::OutputTypeLost);
            } else if !old.types.is_empty() && !new.types.is_subset(&old.types) {
                rules.push(BreakRule::OutputTypeChanged);
            }
            if !old.values.is_empty() && !new.values.is_subset(&old.values) && !new.open {
                rules.push(BreakRule::ClosedOutputEnumGrew);
            }
        }
    }
    rules
}
