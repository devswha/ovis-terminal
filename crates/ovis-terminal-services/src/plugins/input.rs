//! The accepted input-schema subset and argv expansion without a shell.
//!
//! Input reaches the program only as whole argv elements and as validated JSON in the
//! environment; no value is ever spliced into a command line, so `;`, `|` or `$(...)` stay
//! inert bytes of one argument.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::PluginError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaDocument {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    properties: BTreeMap<String, Property>,
    #[serde(default)]
    required: BTreeSet<String>,
    #[serde(rename = "additionalProperties")]
    additional_properties: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Property {
    #[serde(rename = "type")]
    kind: Scalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scalar {
    String,
    Integer,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Arg {
    Literal(String),
    Field(String),
}

/// A checked action definition: an absolute program, an argv template and its input.
#[derive(Debug, Clone)]
pub(super) struct Definition {
    program: PathBuf,
    args: Vec<Arg>,
    properties: BTreeMap<String, Scalar>,
    required: BTreeSet<String>,
}

/// The program, its expanded arguments and the canonical validated input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Expanded {
    pub(super) program: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) input: Value,
}

fn placeholder(arg: &str) -> Option<&str> {
    arg.strip_prefix('{')?.strip_suffix('}')
}

impl Definition {
    /// Checks `argv` and `schema`; `None` when either uses an unsupported form.
    pub(super) fn parse(argv: &[String], schema: &Value) -> Option<Self> {
        let document: SchemaDocument = serde_json::from_value(schema.clone()).ok()?;
        if document.kind != "object"
            || document.additional_properties
            || !document
                .required
                .iter()
                .all(|name| document.properties.contains_key(name))
        {
            return None;
        }
        let (program, rest) = argv.split_first()?;
        let program = PathBuf::from(program);
        if !program.is_absolute()
            || program
                .to_str()
                .is_some_and(|text| text.contains(['{', '}']))
        {
            return None;
        }
        let template = rest
            .iter()
            .map(|arg| match placeholder(arg) {
                Some(field) if document.required.contains(field) => {
                    Some(Arg::Field(field.to_owned()))
                }
                Some(_) => None,
                None if arg.contains(['{', '}']) => None,
                None => Some(Arg::Literal(arg.clone())),
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            program,
            args: template,
            properties: document
                .properties
                .into_iter()
                .map(|(name, property)| (name, property.kind))
                .collect(),
            required: document.required,
        })
    }

    pub(super) fn program(&self) -> &std::path::Path {
        &self.program
    }

    /// Validates `input` and expands the argv template with its canonical values.
    pub(super) fn expand(&self, input: &Value) -> Result<Expanded, PluginError> {
        let invalid = |field: &str| PluginError::InvalidInput {
            field: format!("input{field}"),
        };
        let object = input.as_object().ok_or_else(|| invalid(""))?;
        let mut values = BTreeMap::new();
        for (name, value) in object {
            let kind = self
                .properties
                .get(name)
                .ok_or_else(|| invalid(&format!(".{name}")))?;
            let text = match (kind, value) {
                (Scalar::String, Value::String(text)) if !text.contains('\0') => text.clone(),
                (Scalar::Integer, Value::Number(number)) if number.is_i64() => number.to_string(),
                (Scalar::Boolean, Value::Bool(flag)) => flag.to_string(),
                _ => return Err(invalid(&format!(".{name}"))),
            };
            values.insert(name.as_str(), text);
        }
        if let Some(missing) = self
            .required
            .iter()
            .find(|name| !values.contains_key(name.as_str()))
        {
            return Err(invalid(&format!(".{missing}")));
        }
        let args = self
            .args
            .iter()
            .map(|arg| match arg {
                Arg::Literal(text) => Ok(text.clone()),
                Arg::Field(field) => values
                    .get(field.as_str())
                    .cloned()
                    .ok_or_else(|| invalid(&format!(".{field}"))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Expanded {
            program: self.program.clone(),
            args,
            input: input.clone(),
        })
    }
}
