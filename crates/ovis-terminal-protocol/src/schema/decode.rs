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
/// # Errors
///
/// Returns [`ParamsError`] naming the first offending field; the caller has no
/// typed value to dispatch.
pub fn decode_params<P: DeserializeOwned>(request: &RpcRequest) -> Result<P, ParamsError> {
    serde_path_to_error::deserialize(request.params()).map_err(|error| {
        let path = error.path().to_string();
        let field_path = if path == "." {
            "params".to_owned()
        } else {
            format!("params.{path}")
        };
        ParamsError {
            field_path,
            source: error.into_inner(),
        }
    })
}
