//! Structural comparison of two JSON Schemas in input or output position.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{Break, BreakRule};
use crate::schema::document::COMPONENTS_PREFIX;
use crate::schema::scalar::OPEN_ENUM_EXTENSION;

/// Deepest nesting compared; recursive types stop here instead of looping.
const MAX_DEPTH: usize = 32;

/// Whether a schema describes what clients send or what they receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Position {
    /// Clients send it; the new schema must accept everything the old one did.
    Input,
    /// Clients receive it; the new schema must not produce what the old one could not.
    Output,
}

/// The released and current documents with one schema from each.
#[derive(Debug, Clone, Copy)]
pub(super) struct SchemaPair<'a> {
    old_doc: &'a Value,
    new_doc: &'a Value,
    pub(super) old_value: &'a Value,
    pub(super) new_value: &'a Value,
}

impl<'a> SchemaPair<'a> {
    pub(super) const fn documents(old_doc: &'a Value, new_doc: &'a Value) -> Self {
        Self {
            old_doc,
            new_doc,
            old_value: old_doc,
            new_value: new_doc,
        }
    }

    pub(super) const fn with(self, old_value: &'a Value, new_value: &'a Value) -> Self {
        Self {
            old_value,
            new_value,
            ..self
        }
    }
}

/// Where in a method or event the comparison currently is.
#[derive(Debug, Clone)]
struct Site {
    position: Position,
    path: String,
    depth: usize,
}

impl Site {
    fn child(&self, suffix: &str) -> Self {
        Self {
            position: self.position,
            path: format!("{}{suffix}", self.path),
            depth: self.depth.saturating_add(1),
        }
    }

    fn report(&self, rule: BreakRule, out: &mut Vec<Break>) {
        out.push(Break {
            path: self.path.clone(),
            rule,
        });
    }
}

/// JSON types, enum values, openness and object members of one schema.
#[derive(Default)]
struct Shape<'a> {
    types: BTreeSet<String>,
    values: BTreeSet<String>,
    open: bool,
    objects: Vec<&'a Value>,
    items: Option<&'a Value>,
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

fn collect<'a>(document: &'a Value, schema: &'a Value, found: &mut Shape<'a>) {
    found.open |= is_open(schema);
    let schema = resolve(document, schema);
    found.open |= is_open(schema);
    match schema.get("type") {
        Some(Value::String(name)) => {
            found.types.insert(name.clone());
        }
        Some(Value::Array(names)) => {
            found
                .types
                .extend(names.iter().filter_map(|n| n.as_str().map(str::to_owned)));
        }
        _ => {}
    }
    let enumerated = schema
        .get("enum")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let literals = enumerated.chain(schema.get("const"));
    found
        .values
        .extend(literals.filter_map(|v| v.as_str().map(str::to_owned)));
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

fn shape_of<'a>(document: &'a Value, schema: &'a Value) -> Shape<'a> {
    let mut found = Shape::default();
    collect(document, schema, &mut found);
    found
}

fn names(value: Option<&Value>) -> BTreeSet<&str> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Rules broken by the types and enum values of one schema position.
fn scalar_breaks(old: &Shape<'_>, new: &Shape<'_>, position: Position) -> Vec<BreakRule> {
    let mut rules = Vec::new();
    match position {
        Position::Input => {
            if !new.types.is_empty() && !old.types.is_subset(&new.types) {
                rules.push(BreakRule::InputTypeNarrowed);
            }
            if !new.values.is_empty() && !old.values.is_subset(&new.values) {
                rules.push(BreakRule::InputValuesNarrowed);
            }
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

fn compare_object(pair: SchemaPair<'_>, site: &Site, out: &mut Vec<Break>) {
    let (old, new) = (pair.old_value, pair.new_value);
    let (old_required, new_required) = (names(old.get("required")), names(new.get("required")));
    let old_props = old
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten();
    for (key, old_schema) in old_props {
        let child = site.child(&format!(".{key}"));
        match (
            new.get("properties").and_then(|props| props.get(key)),
            site.position,
        ) {
            (None, Position::Input) => child.report(BreakRule::InputShapeRemoved, out),
            (None, Position::Output) => child.report(BreakRule::OutputPropertyRemoved, out),
            (Some(new_schema), _) => walk(pair.with(old_schema, new_schema), &child, out),
        }
        let dropped = old_required.contains(key.as_str()) && !new_required.contains(key.as_str());
        if site.position == Position::Output && dropped {
            child.report(BreakRule::OutputNoLongerRequired, out);
        }
    }
    if site.position == Position::Input {
        for key in new_required.difference(&old_required) {
            site.child(&format!(".{key}"))
                .report(BreakRule::InputPropertyNewlyRequired, out);
        }
        let closed =
            |schema: &Value| schema.get("additionalProperties") == Some(&Value::Bool(false));
        if closed(new) && !closed(old) {
            site.report(BreakRule::InputClosed, out);
        }
    }
}

fn walk(pair: SchemaPair<'_>, site: &Site, out: &mut Vec<Break>) {
    if site.depth > MAX_DEPTH {
        return;
    }
    let old = shape_of(pair.old_doc, pair.old_value);
    let new = shape_of(pair.new_doc, pair.new_value);
    for rule in scalar_breaks(&old, &new, site.position) {
        site.report(rule, out);
    }
    if let (Some(old_items), Some(new_items)) = (old.items, new.items) {
        walk(pair.with(old_items, new_items), &site.child("[]"), out);
    }
    let single = (old.objects.len() == 1 && new.objects.len() == 1)
        .then(|| new.objects.first())
        .flatten();
    for old_object in &old.objects {
        let key = names(old_object.get("required"));
        let matching = new
            .objects
            .iter()
            .find(|candidate| names(candidate.get("required")) == key);
        match (matching.or(single), site.position) {
            (Some(new_object), _) => {
                compare_object(pair.with(old_object, new_object), &site.child(""), out);
            }
            (None, Position::Input) => site.report(BreakRule::InputShapeRemoved, out),
            (None, Position::Output) => {}
        }
    }
}

/// Lists the breaks between one released schema and its current form.
pub(super) fn compare(pair: SchemaPair<'_>, position: Position, path: &str) -> Vec<Break> {
    let mut out = Vec::new();
    walk(
        pair,
        &Site {
            position,
            path: path.to_owned(),
            depth: 0,
        },
        &mut out,
    );
    out
}
