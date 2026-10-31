//! Bounded opaque document values and advertised retention floors (plan todo 50).
//!
//! A document value is limited by the byte length of its compact JSON encoding. Every
//! retention class advertises its bound; eviction advances a floor, and a read below it is
//! `expired` or a `gap`, never an empty success.

use serde_json::Value;

mod retention;

pub use retention::{
    Retained, RetentionClass, RetentionFloor, RetentionFloors, RetentionLimit, RetentionPolicy,
    RetentionPosition, RetentionQuantity, RetentionUnit,
};

use crate::schema::ids::DocName;
use crate::schema::methods::doc::{DocEntry, DocRecord, DocSetResult};
use crate::schema::scalar::{ByteCount, EventSeq, RecordVersion};

/// Longest document name, in bytes.
pub const DOC_NAME_MAX_BYTES: usize = 128;

/// A document name or value outside its advertised bounds.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DocumentError {
    /// The name is empty, longer than [`DOC_NAME_MAX_BYTES`] or uses another byte.
    #[error("a document name is 1 to 128 bytes of ASCII letters, digits, '.', '_' or '-'")]
    InvalidName,
    /// The compact JSON encoding is longer than the advertised bound.
    #[error("the encoded value has {actual} bytes, above the {limit}-byte bound")]
    TooLarge {
        /// Advertised bound in bytes.
        limit: u64,
        /// Encoded length in bytes.
        actual: u64,
    },
    /// Stored bytes are not one JSON document.
    #[error("stored document bytes do not decode")]
    Malformed(#[source] serde_json::Error),
}

impl DocumentError {
    /// Stable machine token.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidName => "invalid_name",
            Self::TooLarge { .. } => "too_large",
            Self::Malformed(_) => "invalid_record",
        }
    }

    /// Recovery guidance.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidName => "use 1 to 128 bytes of ASCII letters, digits, '.', '_' or '-'",
            Self::TooLarge { .. } => "shrink the value below the advertised doc_max_bytes",
            Self::Malformed(_) => "inspect the stored document; it was not replaced",
        }
    }
}

/// Accepts a name of 1 to 128 bytes from `[A-Za-z0-9._-]`.
///
/// # Errors
///
/// Returns [`DocumentError::InvalidName`] for any other name.
pub fn validate_name(name: &DocName) -> Result<(), DocumentError> {
    let bytes = name.as_str().as_bytes();
    let allowed = |byte: &u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-');
    if bytes.is_empty() || bytes.len() > DOC_NAME_MAX_BYTES || !bytes.iter().all(allowed) {
        return Err(DocumentError::InvalidName);
    }
    Ok(())
}

/// An opaque JSON value whose compact encoding fits a byte bound; null deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocValue {
    value: Value,
    encoded: Vec<u8>,
}

impl DocValue {
    /// Encodes `value` and checks it against `limit`.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::TooLarge`] above the bound.
    pub fn bounded(value: Value, limit: ByteCount) -> Result<Self, DocumentError> {
        let encoded = serde_json::to_vec(&value).map_err(DocumentError::Malformed)?;
        within(encoded.len(), limit)?;
        Ok(Self { value, encoded })
    }

    /// Decodes stored bytes, which must still fit `limit`.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::Malformed`] or [`DocumentError::TooLarge`].
    pub fn decode(encoded: Vec<u8>, limit: ByteCount) -> Result<Self, DocumentError> {
        within(encoded.len(), limit)?;
        let value = serde_json::from_slice(&encoded).map_err(DocumentError::Malformed)?;
        Ok(Self { value, encoded })
    }

    /// The JSON value.
    pub const fn value(&self) -> &Value {
        &self.value
    }

    /// The compact encoding that was measured.
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// Whether storing this value deletes the document while keeping its version.
    pub const fn deletes(&self) -> bool {
        matches!(self.value, Value::Null)
    }

    /// Releases the JSON value.
    pub fn into_value(self) -> Value {
        self.value
    }
}

fn within(length: usize, limit: ByteCount) -> Result<(), DocumentError> {
    let actual = u64::try_from(length).unwrap_or(u64::MAX);
    if actual > limit.get() {
        return Err(DocumentError::TooLarge {
            limit: limit.get(),
            actual,
        });
    }
    Ok(())
}

impl DocRecord {
    /// A document at `version`; an absent document is a null value at version zero.
    pub const fn new(name: DocName, value: Value, version: RecordVersion) -> Self {
        Self {
            name,
            value,
            version,
        }
    }
}

impl DocEntry {
    /// The `doc.changed` payload: a name and its new version, never the value.
    pub const fn new(name: DocName, version: RecordVersion) -> Self {
        Self { name, version }
    }
}

impl DocSetResult {
    /// The committed version and the sequence of its change event.
    pub const fn new(version: RecordVersion, seq: EventSeq) -> Self {
        Self { version, seq }
    }
}
