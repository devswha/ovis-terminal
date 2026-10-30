//! Output comparison: clients must still receive every required field and decodable value.

use std::collections::BTreeSet;

use serde_json::Value;

use super::facts::{Alt, Facts, alternatives, facts};
use super::shape::{SchemaPair, Site};
use super::{Break, BreakRule};
use crate::schema::error::SchemaError;

/// Types, literals, literal-free types and openness across every alternative of one output schema.
struct Union {
    types: Option<BTreeSet<String>>,
    literals: Vec<Value>,
    free: Option<BTreeSet<String>>,
    open: bool,
}

fn union(all: &[(Alt<'_>, Facts<'_>)]) -> Union {
    let types = all.iter().try_fold(BTreeSet::new(), |mut acc, (_, f)| {
        acc.extend(f.types.as_ref()?.iter().cloned());
        Some(acc)
    });
    let literals = all
        .iter()
        .filter_map(|(_, f)| f.literals.as_ref())
        .flatten()
        .cloned()
        .collect();
    let free = all.iter().filter(|(_, f)| f.literals.is_none()).try_fold(
        BTreeSet::new(),
        |mut acc, (_, f)| {
            acc.extend(f.types.as_ref()?.iter().cloned());
            Some(acc)
        },
    );
    Union {
        types,
        literals,
        free,
        open: all.iter().any(|(_, f)| f.open),
    }
}

/// JSON Schema type names a literal value belongs to.
fn literal_types(value: &Value) -> &'static [&'static str] {
    match value {
        Value::Null => &["null"],
        Value::Bool(_) => &["boolean"],
        Value::Number(n) if n.is_f64() => &["number"],
        Value::Number(_) => &["integer", "number"],
        Value::String(_) => &["string"],
        Value::Array(_) => &["array"],
        Value::Object(_) => &["object"],
    }
}

/// Whether the released output could already produce `value`.
fn produced(old: &Union, value: &Value) -> bool {
    old.literals.contains(value)
        || old
            .free
            .as_ref()
            .is_none_or(|free| literal_types(value).iter().any(|t| free.contains(*t)))
}

fn scalar_rules(old: &Union, new: &Union) -> Vec<BreakRule> {
    let mut rules = Vec::new();
    match (&old.types, &new.types) {
        (Some(_), None) => rules.push(BreakRule::OutputTypeLost),
        (Some(was), Some(now)) if !now.is_subset(was) => rules.push(BreakRule::OutputTypeChanged),
        _ => {}
    }
    let new_value = new.literals.iter().any(|value| !produced(old, value));
    let new_free = match (&old.free, &new.free) {
        (Some(was), Some(now)) => !now.is_subset(was),
        (Some(_), None) => true,
        (None, _) => false,
    };
    if (new_value || new_free) && !new.open {
        rules.push(BreakRule::ClosedOutputEnumGrew);
    }
    rules
}

fn property_names<'a>(f: &Facts<'a>) -> BTreeSet<&'a str> {
    f.properties.keys().copied().collect()
}

/// The current object alternative that continues `old_objects[index]`: same
/// property names, else same required names, else the same position.
fn counterpart<'b, 'a>(
    old_objects: &[(Alt<'a>, Facts<'a>)],
    index: usize,
    new: &'b [(Alt<'a>, Facts<'a>)],
) -> Option<&'b (Alt<'a>, Facts<'a>)> {
    let (_, old) = old_objects.get(index)?;
    new.iter()
        .find(|(_, f)| property_names(f) == property_names(old))
        .or_else(|| new.iter().find(|(_, f)| f.required == old.required))
        .or_else(|| {
            (new.len() == old_objects.len())
                .then(|| new.get(index))
                .flatten()
        })
}

fn object_breaks(
    pair: SchemaPair<'_>,
    (old, new): (&(Alt<'_>, Facts<'_>), &(Alt<'_>, Facts<'_>)),
    site: &Site,
) -> Result<Vec<Break>, SchemaError> {
    let mut out = Vec::new();
    for (key, old_schema) in &old.1.properties {
        let child = site.descend(&format!(".{key}"), (&old.0, &new.0));
        match new.1.properties.get(key) {
            None => child.report(BreakRule::OutputPropertyRemoved, &mut out),
            Some(new_schema) => out.extend(breaks(pair.with(old_schema, new_schema), &child)?),
        }
        if old.1.required.contains(key) && !new.1.required.contains(key) {
            child.report(BreakRule::OutputNoLongerRequired, &mut out);
        }
    }
    Ok(out)
}

/// Lists output breaks of one released schema against its current form.
///
/// # Errors
///
/// Returns a typed [`SchemaError`] for reference cycles, excessive depth or dangling references.
pub(super) fn breaks(pair: SchemaPair<'_>, site: &Site) -> Result<Vec<Break>, SchemaError> {
    let olds = with_facts(alternatives(pair.old_doc, pair.old_value, &site.old)?);
    let news = with_facts(alternatives(pair.new_doc, pair.new_value, &site.new)?);
    let mut out = Vec::new();
    for rule in scalar_rules(&union(&olds), &union(&news)) {
        site.report(rule, &mut out);
    }
    let (old_objects, new_objects) = (objects(&olds), objects(&news));
    for index in 0..old_objects.len() {
        match (
            old_objects.get(index),
            counterpart(&old_objects, index, &new_objects),
        ) {
            (Some(old), Some(new)) => out.extend(object_breaks(pair, (old, new), site)?),
            (Some(_), None) => site.report(BreakRule::OutputVariantRemoved, &mut out),
            (None, _) => {}
        }
    }
    if let (Some((old_alt, old_items)), Some((new_alt, new_items))) = (items(&olds), items(&news)) {
        out.extend(breaks(
            pair.with(old_items, new_items),
            &site.descend("[]", (old_alt, new_alt)),
        )?);
    }
    Ok(out)
}

fn with_facts(alts: Vec<Alt<'_>>) -> Vec<(Alt<'_>, Facts<'_>)> {
    alts.into_iter()
        .map(|alt| {
            let found = facts(&alt);
            (alt, found)
        })
        .collect()
}

fn objects<'a>(all: &[(Alt<'a>, Facts<'a>)]) -> Vec<(Alt<'a>, Facts<'a>)> {
    all.iter()
        .filter(|(_, f)| !f.properties.is_empty())
        .cloned()
        .collect()
}

fn items<'b, 'a>(all: &'b [(Alt<'a>, Facts<'a>)]) -> Option<(&'b Alt<'a>, &'a Value)> {
    all.iter()
        .find_map(|(alt, f)| f.items.map(|items| (alt, items)))
}
