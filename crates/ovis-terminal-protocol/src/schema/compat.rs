//! Released-baseline gate: inputs only widen, outputs only grow.

mod entries;
mod facts;
mod rule;
mod shape;

use entries::{Section, entries, member, params};
pub use rule::BreakRule;
use serde_json::Value;
use shape::{Position, SchemaPair, compare};

use super::error::SchemaError;

/// One breaking change with the path where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Break {
    /// Location such as `terminal.get(target).terminal_id` or `terminal.get ->.state`.
    pub path: String,
    /// Violated rule.
    pub rule: BreakRule,
}

fn is_required(param: &Value) -> bool {
    param.get("required").and_then(Value::as_bool) == Some(true)
}

fn compare_params(
    pair: SchemaPair<'_>,
    method: &str,
    out: &mut Vec<Break>,
) -> Result<(), SchemaError> {
    let (old, new) = (
        params(pair.old_value, method)?,
        params(pair.new_value, method)?,
    );
    for (name, old_param) in &old {
        let path = format!("{method}({name})");
        let Some((_, new_param)) = new.iter().find(|(candidate, _)| candidate == name) else {
            out.push(Break {
                path,
                rule: BreakRule::ParamRemoved,
            });
            continue;
        };
        if is_required(new_param) && !is_required(old_param) {
            out.push(Break {
                path: path.clone(),
                rule: BreakRule::ParamNewlyRequired,
            });
        }
        let schemas = pair.with(
            member(old_param, "/schema", &path)?,
            member(new_param, "/schema", &path)?,
        );
        out.extend(compare(schemas, Position::Input, &path));
    }
    for (name, _) in new.iter().filter(|(_, param)| is_required(param)) {
        if !old.iter().any(|(candidate, _)| candidate == name) {
            out.push(Break {
                path: format!("{method}({name})"),
                rule: BreakRule::RequiredParamAdded,
            });
        }
    }
    Ok(())
}

fn compare_methods(
    released: &Value,
    current: &Value,
    out: &mut Vec<Break>,
) -> Result<(), SchemaError> {
    let current_methods = entries(current, Section::Methods)?;
    for (method, old) in entries(released, Section::Methods)? {
        let Some((_, new)) = current_methods.iter().find(|(name, _)| *name == method) else {
            out.push(Break {
                path: method.to_owned(),
                rule: BreakRule::MethodRemoved,
            });
            continue;
        };
        let pair = SchemaPair::documents(released, current).with(old, new);
        compare_params(pair, method, out)?;
        let results = pair.with(
            member(old, "/result/schema", method)?,
            member(new, "/result/schema", method)?,
        );
        out.extend(compare(results, Position::Output, &format!("{method} ->")));
    }
    Ok(())
}

/// Compares every event or notification of `section` as client-received output.
fn compare_signals(
    pair: SchemaPair<'_>,
    section: Section,
    out: &mut Vec<Break>,
) -> Result<(), SchemaError> {
    let current = entries(pair.new_value, section)?;
    let removed = match section {
        Section::Notifications => BreakRule::NotificationRemoved,
        Section::Events | Section::Methods => BreakRule::EventRemoved,
    };
    for (name, old) in entries(pair.old_value, section)? {
        match current.iter().find(|(candidate, _)| *candidate == name) {
            None => out.push(Break {
                path: name.to_owned(),
                rule: removed,
            }),
            Some((_, new)) => {
                let schemas =
                    pair.with(member(old, "/schema", name)?, member(new, "/schema", name)?);
                out.extend(compare(schemas, Position::Output, &format!("{name} ->")));
            }
        }
    }
    Ok(())
}

/// Lists every change from `released` to `current` that breaks a released client.
///
/// Methods, events and notifications are all compared; inputs may only widen
/// and outputs may only grow.
///
/// # Errors
///
/// Returns [`SchemaError::MalformedBaseline`] when either document lacks a section,
/// or an entry lacks its name or carries a key its section does not define.
pub fn breaking_changes(released: &Value, current: &Value) -> Result<Vec<Break>, SchemaError> {
    let mut out = Vec::new();
    compare_methods(released, current, &mut out)?;
    let documents = SchemaPair::documents(released, current);
    compare_signals(documents, Section::Events, &mut out)?;
    compare_signals(documents, Section::Notifications, &mut out)?;
    Ok(out)
}

/// Fails when `current` breaks any client of `released`.
///
/// # Errors
///
/// Returns [`SchemaError::Incompatible`] listing every break, or
/// [`SchemaError::MalformedBaseline`] when a document is not a well-formed schema.
pub fn check_compatibility(released: &Value, current: &Value) -> Result<(), SchemaError> {
    let breaks = breaking_changes(released, current)?;
    if breaks.is_empty() {
        Ok(())
    } else {
        Err(SchemaError::Incompatible { breaks })
    }
}
