//! Validated `UUIDv7` requests and canonical parameter fingerprints.

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::MethodName;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::OperationError;

#[cfg(test)]
mod tests;

/// Fingerprint of canonical method parameters, not raw request formatting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ParamsDigest(Vec<u8>);

/// An operation identity, method and already schema-validated parameter object.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    /// Client-minted `UUIDv7` operation identity.
    pub(super) op_id: OpId,
    pub(super) method: MethodName,
    pub(super) params: ParamsDigest,
    pub(super) minted_at: TimestampMs,
}

impl Request {
    /// Borrows the validated operation identity without allowing it to be replaced.
    pub const fn op_id(&self) -> &OpId {
        &self.op_id
    }

    /// Canonicalizes parameters after the method's closed schema has parsed them.
    ///
    /// The operation ID is separate from the method parameters; callers must
    /// remove envelope metadata before constructing this request.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for a noncanonical `UUIDv7`, empty method or non-object
    /// params, and `too_large` above the 256 KiB canonical parameter bound.
    pub fn new(op_id: OpId, method: String, mut params: Value) -> Result<Self, OperationError> {
        let minted_at = minted_at(&op_id)?;
        if method.is_empty() || method.len() > 128 || !params.is_object() {
            return Err(OperationError::InvalidParams);
        }
        canonicalize(&mut params);
        let bytes = serde_json::to_vec(&params)?;
        if bytes.len() > 256 << 10 {
            return Err(OperationError::TooLarge);
        }
        Ok(Self {
            op_id,
            method: MethodName::new(method),
            params: ParamsDigest(Sha256::digest(bytes).to_vec()),
            minted_at,
        })
    }
}

pub(super) fn minted_at(id: &OpId) -> Result<TimestampMs, OperationError> {
    // The protocol identity exposes its wire representation, not its private string.
    let text: String = serde_json::from_value(serde_json::to_value(id)?)?;
    let uuid = Uuid::parse_str(&text).map_err(OperationError::InvalidId)?;
    if uuid.get_version_num() != 7
        || uuid.get_variant() != uuid::Variant::RFC4122
        || uuid.to_string() != text
    {
        return Err(OperationError::InvalidParams);
    }
    let (seconds, nanos) = uuid
        .get_timestamp()
        .ok_or(OperationError::InvalidParams)?
        .to_unix();
    let millis = seconds
        .checked_mul(1000)
        .and_then(|value| value.checked_add(u64::from(nanos / 1_000_000)))
        .ok_or(OperationError::InvalidParams)?;
    Ok(TimestampMs::new(millis))
}

fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.sort_keys();
            for field in fields.values_mut() {
                canonicalize(field);
            }
        }
        Value::Array(values) => {
            for field in values {
                canonicalize(field);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
