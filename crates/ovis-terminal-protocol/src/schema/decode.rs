//! Parse-once decoding of method parameters into their closed typed form.

use serde::de::{self, DeserializeOwned, Visitor};
use serde::forward_to_deserialize_any;
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

/// The first top-level key of `params` that `P` does not declare, when `P` is a struct.
fn unknown_key<P: DeserializeOwned>(params: &serde_json::Value) -> Option<String> {
    let mut declared = None;
    // The probe always fails; it only records the field list serde hands to a struct.
    let probed = P::deserialize(FieldNames(&mut declared));
    let fields = declared.filter(|_| probed.is_err())?;
    params
        .as_object()?
        .keys()
        .find(|key| !fields.contains(&key.as_str()))
        .cloned()
}

/// A deserializer that captures the field names a derived struct declares.
struct FieldNames<'a>(&'a mut Option<&'static [&'static str]>);

impl<'de> de::Deserializer<'de> for FieldNames<'_> {
    type Error = serde_json::Error;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("params are not a struct"))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        *self.0 = Some(fields);
        Err(de::Error::custom("struct fields captured"))
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct map enum
        identifier ignored_any
    }
}

/// Name of the absent required field. serde names it only in its message,
/// while the tracked path stops at the enclosing object.
fn missing_field(error: &serde_json::Error) -> Option<String> {
    let message = error.to_string();
    let rest = message.strip_prefix("missing field `")?;
    rest.split('`').next().map(str::to_owned)
}
