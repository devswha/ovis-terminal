//! Input comparison: every value the released schema accepted must still be accepted.

use serde_json::Value;

use super::facts::{Alt, Facts, LOWER_BOUNDS, UPPER_BOUNDS, alternatives, facts};
use super::shape::{SchemaPair, Site};
use super::{Break, BreakRule};
use crate::schema::error::SchemaError;

/// Schema accepting every value; stands in for an absent subschema.
const ANY: Value = Value::Bool(true);

/// JSON types a bound keyword constrains.
fn constrained_types(keyword: &str) -> &'static [&'static str] {
    match keyword {
        "maximum" | "exclusiveMaximum" | "minimum" | "exclusiveMinimum" => &["integer", "number"],
        "maxLength" | "minLength" | "pattern" => &["string"],
        "maxItems" | "minItems" => &["array"],
        _ => &["object"],
    }
}

/// Whether `facts` can hold a value of a type the keyword constrains.
fn applies(facts: &Facts<'_>, keyword: &str) -> bool {
    facts.types.as_ref().is_none_or(|types| {
        constrained_types(keyword)
            .iter()
            .any(|t| types.contains(*t))
    })
}

/// Rules broken by types, literals, bounds and patterns of one alternative pair.
fn scalar_rules(old: &Facts<'_>, new: &Facts<'_>) -> Vec<BreakRule> {
    let mut rules = Vec::new();
    if let Some(accepted) = &new.types
        && old
            .types
            .as_ref()
            .is_none_or(|types| !types.is_subset(accepted))
    {
        rules.push(BreakRule::InputTypeNarrowed);
    }
    if let Some(accepted) = &new.literals {
        let kept = old
            .literals
            .as_ref()
            .is_some_and(|values| values.iter().all(|v| accepted.contains(v)));
        if !kept {
            rules.push(BreakRule::InputValuesNarrowed);
        }
    }
    let tighter_upper = UPPER_BOUNDS
        .iter()
        .filter(|k| applies(new, k))
        .any(|keyword| {
            new.uppers
                .get(keyword)
                .is_some_and(|n| old.uppers.get(keyword).is_none_or(|o| n < o))
        });
    let tighter_lower = LOWER_BOUNDS
        .iter()
        .filter(|k| applies(new, k))
        .any(|keyword| {
            new.lowers
                .get(keyword)
                .is_some_and(|n| old.lowers.get(keyword).is_none_or(|o| n > o))
        });
    if tighter_upper || tighter_lower {
        rules.push(BreakRule::InputBoundNarrowed);
    }
    if applies(new, "pattern") && !new.patterns.is_subset(&old.patterns) {
        rules.push(BreakRule::InputPatternChanged);
    }
    rules
}

/// Breaks of the object members of one alternative pair.
fn member_breaks(
    pair: SchemaPair<'_>,
    (old, new): (&Alt<'_>, &Alt<'_>),
    site: &Site,
) -> Result<Vec<Break>, SchemaError> {
    let (fo, fnew) = (facts(old), facts(new));
    let mut out = Vec::new();
    for (key, old_schema) in &fo.properties {
        let child = site.descend(&format!(".{key}"), (old, new));
        match fnew.properties.get(key).copied().or(fnew.additional) {
            Some(new_schema) => out.extend(breaks(pair.with(old_schema, new_schema), &child)?),
            None if fnew.closed => child.report(BreakRule::InputShapeRemoved, &mut out),
            None => {}
        }
    }
    for (pattern, old_schema) in &fo.pattern_properties {
        let child = site.descend(&format!(".[{pattern}]"), (old, new));
        match fnew
            .pattern_properties
            .get(pattern)
            .copied()
            .or(fnew.additional)
        {
            Some(new_schema) => out.extend(breaks(pair.with(old_schema, new_schema), &child)?),
            None if fnew.closed => child.report(BreakRule::InputShapeRemoved, &mut out),
            None => {}
        }
    }
    for key in fnew.required.difference(&fo.required) {
        site.descend(&format!(".{key}"), (old, new))
            .report(BreakRule::InputPropertyNewlyRequired, &mut out);
    }
    if fnew.closed && !fo.closed {
        site.report(BreakRule::InputClosed, &mut out);
    }
    if let Some(new_extra) = fnew.additional {
        let old_extra = fo.additional.unwrap_or(&ANY);
        out.extend(breaks(
            pair.with(old_extra, new_extra),
            &site.descend(".*", (old, new)),
        )?);
    }
    if fo.items.is_some() || fnew.items.is_some() {
        let items = pair.with(fo.items.unwrap_or(&ANY), fnew.items.unwrap_or(&ANY));
        out.extend(breaks(items, &site.descend("[]", (old, new)))?);
    }
    Ok(out)
}

fn alt_breaks(
    pair: SchemaPair<'_>,
    alts: (&Alt<'_>, &Alt<'_>),
    site: &Site,
) -> Result<Vec<Break>, SchemaError> {
    let (old, new) = (facts(alts.0), facts(alts.1));
    let mut out = Vec::new();
    for rule in scalar_rules(&old, &new) {
        site.report(rule, &mut out);
    }
    out.extend(member_breaks(pair, alts, site)?);
    Ok(out)
}

/// Lists input breaks: each released alternative must be covered by a current one.
///
/// # Errors
///
/// Returns a typed [`SchemaError`] for reference cycles, excessive depth or dangling references.
pub(super) fn breaks(pair: SchemaPair<'_>, site: &Site) -> Result<Vec<Break>, SchemaError> {
    let olds = alternatives(pair.old_doc, pair.old_value, &site.old)?;
    let news = alternatives(pair.new_doc, pair.new_value, &site.new)?;
    let mut out = Vec::new();
    for old in &olds {
        let mut closest: Option<Vec<Break>> = None;
        for new in &news {
            let found = alt_breaks(pair, (old, new), site)?;
            if closest.as_ref().is_none_or(|best| found.len() < best.len()) {
                closest = Some(found);
            }
            if closest.as_ref().is_some_and(Vec::is_empty) {
                break;
            }
        }
        match closest {
            Some(found) => out.extend(found),
            None => site.report(BreakRule::InputShapeRemoved, &mut out),
        }
    }
    Ok(out)
}
