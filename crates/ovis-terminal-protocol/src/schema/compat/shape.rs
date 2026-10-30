//! Structural comparison of two JSON Schemas in input or output position.

use std::collections::BTreeSet;

use serde_json::Value;

use super::facts::{scalar_breaks, shape_of};
use super::{Break, BreakRule};

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

/// Where in a method, event or notification the comparison currently is.
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

fn names(value: Option<&Value>) -> BTreeSet<&str> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn property_names(object: &Value) -> BTreeSet<&str> {
    object
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(key, _)| key.as_str())
        .collect()
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

/// The current union member that continues `old`: same property names, else
/// same required names, else the member at the same position of an equally long union.
fn counterpart<'a>(old_members: &[&Value], index: usize, new: &[&'a Value]) -> Option<&'a Value> {
    let old = old_members.get(index)?;
    let by_properties = new
        .iter()
        .find(|candidate| property_names(candidate) == property_names(old));
    let by_required = || {
        new.iter()
            .find(|candidate| names(candidate.get("required")) == names(old.get("required")))
    };
    let by_position = || {
        (new.len() == old_members.len())
            .then(|| new.get(index))
            .flatten()
    };
    by_properties
        .or_else(by_required)
        .or_else(by_position)
        .copied()
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
    for (index, old_object) in old.objects.iter().enumerate() {
        match (
            counterpart(&old.objects, index, &new.objects),
            site.position,
        ) {
            (Some(new_object), _) => {
                compare_object(pair.with(old_object, new_object), &site.child(""), out);
            }
            (None, Position::Input) => site.report(BreakRule::InputShapeRemoved, out),
            (None, Position::Output) => site.report(BreakRule::OutputVariantRemoved, out),
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
