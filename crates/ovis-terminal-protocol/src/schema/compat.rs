//! Released-baseline gate: inputs only widen, outputs only grow.

mod shape;

use serde_json::Value;
use shape::{Position, SchemaPair, compare};

use super::error::SchemaError;

/// Rule a change violates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BreakRule {
    /// A released method is gone.
    MethodRemoved,
    /// A released parameter is gone.
    ParamRemoved,
    /// An optional parameter became required.
    ParamNewlyRequired,
    /// A new parameter is required.
    RequiredParamAdded,
    /// An input accepts fewer JSON types than before.
    InputTypeNarrowed,
    /// An input accepts fewer enum values than before.
    InputValuesNarrowed,
    /// An input object or union lost a property or variant.
    InputShapeRemoved,
    /// An input object property became required.
    InputPropertyNewlyRequired,
    /// An input object stopped accepting additional properties.
    InputClosed,
    /// An output can now carry a JSON type it never carried.
    OutputTypeChanged,
    /// An output lost its declared type.
    OutputTypeLost,
    /// A released output property is gone.
    OutputPropertyRemoved,
    /// A required output property became optional.
    OutputNoLongerRequired,
    /// A closed output enum gained a value clients cannot decode.
    ClosedOutputEnumGrew,
    /// A released event type is gone.
    EventRemoved,
}

impl BreakRule {
    /// Returns the stable machine token of this rule.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::MethodRemoved => "method_removed",
            Self::ParamRemoved => "param_removed",
            Self::ParamNewlyRequired => "param_newly_required",
            Self::RequiredParamAdded => "required_param_added",
            Self::InputTypeNarrowed => "input_type_narrowed",
            Self::InputValuesNarrowed => "input_values_narrowed",
            Self::InputShapeRemoved => "input_shape_removed",
            Self::InputPropertyNewlyRequired => "input_property_newly_required",
            Self::InputClosed => "input_closed",
            Self::OutputTypeChanged => "output_type_changed",
            Self::OutputTypeLost => "output_type_lost",
            Self::OutputPropertyRemoved => "output_property_removed",
            Self::OutputNoLongerRequired => "output_no_longer_required",
            Self::ClosedOutputEnumGrew => "closed_output_enum_grew",
            Self::EventRemoved => "event_removed",
        }
    }
}

/// One breaking change with the path where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Break {
    /// Location such as `terminal.get(target).terminal_id` or `terminal.get ->.state`.
    pub path: String,
    /// Violated rule.
    pub rule: BreakRule,
}

fn named<'a>(
    document: &'a Value,
    section: &'static str,
) -> Result<Vec<(&'a str, &'a Value)>, SchemaError> {
    let entries = document
        .get(section)
        .and_then(Value::as_array)
        .ok_or(SchemaError::Malformed { section })?;
    Ok(entries
        .iter()
        .filter_map(|entry| Some((entry.get("name")?.as_str()?, entry)))
        .collect())
}

/// The member at `pointer`, or a typed failure naming the missing section.
fn member<'a>(
    value: &'a Value,
    pointer: &str,
    section: &'static str,
) -> Result<&'a Value, SchemaError> {
    value
        .pointer(pointer)
        .ok_or(SchemaError::Malformed { section })
}

fn is_required(param: &Value) -> bool {
    param.get("required").and_then(Value::as_bool) == Some(true)
}

/// By-name parameters of one method object.
fn params(method: &Value) -> Result<Vec<(&str, &Value)>, SchemaError> {
    let malformed = SchemaError::Malformed {
        section: "method params",
    };
    let entries = method
        .get("params")
        .and_then(Value::as_array)
        .ok_or(malformed)?;
    entries
        .iter()
        .map(|param| {
            let name = param.get("name").and_then(Value::as_str);
            name.map(|name| (name, param))
                .ok_or(SchemaError::Malformed {
                    section: "param name",
                })
        })
        .collect()
}

fn compare_params(
    pair: SchemaPair<'_>,
    method: &str,
    out: &mut Vec<Break>,
) -> Result<(), SchemaError> {
    let (old, new) = (params(pair.old_value)?, params(pair.new_value)?);
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
        let (old_schema, new_schema) = (
            member(old_param, "/schema", "param schema")?,
            member(new_param, "/schema", "param schema")?,
        );
        let schemas = pair.with(old_schema, new_schema);
        out.extend(compare(schemas, Position::Input, &path));
    }
    for (name, _) in new.iter().filter(|(_, param)| is_required(param)) {
        if !old.iter().any(|(candidate, _)| candidate == name) {
            let path = format!("{method}({name})");
            out.push(Break {
                path,
                rule: BreakRule::RequiredParamAdded,
            });
        }
    }
    Ok(())
}

/// Lists every change from `released` to `current` that breaks a released client.
///
/// # Errors
///
/// Returns [`SchemaError::Malformed`] when either document lacks its methods or events.
pub fn breaking_changes(released: &Value, current: &Value) -> Result<Vec<Break>, SchemaError> {
    let mut out = Vec::new();
    let current_methods = named(current, "methods")?;
    for (method, old) in named(released, "methods")? {
        let Some((_, new)) = current_methods.iter().find(|(name, _)| *name == method) else {
            out.push(Break {
                path: method.to_owned(),
                rule: BreakRule::MethodRemoved,
            });
            continue;
        };
        let pair = SchemaPair::documents(released, current).with(old, new);
        compare_params(pair, method, &mut out)?;
        let section = "result schema";
        let results = pair.with(
            member(old, "/result/schema", section)?,
            member(new, "/result/schema", section)?,
        );
        out.extend(compare(results, Position::Output, &format!("{method} ->")));
    }
    let current_events = named(current, "x-events")?;
    for (event, old) in named(released, "x-events")? {
        match current_events.iter().find(|(name, _)| *name == event) {
            None => out.push(Break {
                path: event.to_owned(),
                rule: BreakRule::EventRemoved,
            }),
            Some((_, new)) => {
                let (old_data, new_data) = (
                    member(old, "/schema", "event schema")?,
                    member(new, "/schema", "event schema")?,
                );
                let pair = SchemaPair::documents(released, current).with(old_data, new_data);
                out.extend(compare(pair, Position::Output, &format!("event {event}")));
            }
        }
    }
    Ok(out)
}

/// Fails when `current` breaks any client of `released`.
///
/// # Errors
///
/// Returns [`SchemaError::Incompatible`] listing every break, or
/// [`SchemaError::Malformed`] when a document lacks a required section.
pub fn check_compatibility(released: &Value, current: &Value) -> Result<(), SchemaError> {
    let breaks = breaking_changes(released, current)?;
    if breaks.is_empty() {
        Ok(())
    } else {
        Err(SchemaError::Incompatible { breaks })
    }
}
