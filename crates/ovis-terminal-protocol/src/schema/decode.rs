//! Parse-once decoding of method parameters into their closed typed form.

use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::errors::BoundaryError;
use crate::rpc::RpcRequest;

/// Parameters that do not match the method's closed schema; nothing was dispatched.
#[derive(Debug, Error)]
#[error("method parameters violate the closed schema at {field_path}")]
pub struct ParamsError {
    field_path: String,
    #[source]
    source: serde_json::Error,
}

impl ParamsError {
    /// Returns the stable machine token clients branch on.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        "invalid_params"
    }

    /// Returns recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        "correct the field at field_path; unknown fields are never ignored"
    }

    /// Returns the dotted path of the offending field, rooted at `params`.
    #[must_use]
    pub fn field_path(&self) -> &str {
        &self.field_path
    }
}

impl From<ParamsError> for BoundaryError {
    fn from(error: ParamsError) -> Self {
        Self::invalid_params(Some(&error.field_path))
    }
}

/// Decodes a request's params into `P`, rejecting unknown fields, wrong types
/// and missing required fields with the offending path.
///
/// When the params are refused and a top-level key is not a field of `P`, that key is
/// the offending path: a misspelled target such as `pane_id_typo` is named itself,
/// not the required field its typo left absent.
///
/// # Errors
///
/// Returns [`ParamsError`] naming the first offending field; the caller has no
/// typed value to dispatch.
pub fn decode_params<P: DeserializeOwned>(request: &RpcRequest) -> Result<P, ParamsError> {
    serde_path_to_error::deserialize(request.params()).map_err(|error| {
        let path = error.path().to_string();
        let source = error.into_inner();
        if let Some(key) = unknown_key::<P>(request.params()) {
            return ParamsError {
                field_path: format!("params.{key}"),
                source,
            };
        }
        let mut segments = vec!["params".to_owned()];
        if path != "." {
            segments.push(path);
        }
        segments.extend(missing_field(&source));
        ParamsError {
            field_path: segments.join("."),
            source,
        }
    })
}

/// The first top-level key of `params` that `P` refuses as unknown. Each key is decoded
/// alone, so the refusal cannot come from another member; serde names an unknown field
/// only in its message, as it does a missing one.
fn unknown_key<P: DeserializeOwned>(params: &serde_json::Value) -> Option<String> {
    params.as_object()?.iter().find_map(|(key, value)| {
        let alone =
            serde_json::Value::Object(std::iter::once((key.clone(), value.clone())).collect());
        let refused = serde_json::from_value::<P>(alone).err()?;
        refused
            .to_string()
            .starts_with(&format!("unknown field `{key}`"))
            .then(|| key.clone())
    })
}

/// Name of the absent required field. serde names it only in its message,
/// while the tracked path stops at the enclosing object.
fn missing_field(error: &serde_json::Error) -> Option<String> {
    let message = error.to_string();
    let rest = message.strip_prefix("missing field `")?;
    rest.split('`').next().map(str::to_owned)
}
