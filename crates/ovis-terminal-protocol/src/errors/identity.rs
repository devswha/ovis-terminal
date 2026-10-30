//! Identities a boundary error can refer to.

use serde::{Deserialize, Serialize};

/// Opaque stable identity of the resource an error refers to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TargetId(String);

impl TargetId {
    /// Wraps a resource identity string.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }
}

/// Client-minted identity of an operation whose effects may have begun.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpId(String);

impl OpId {
    /// Wraps an operation identity string.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }
}

/// Dotted path to the offending input field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(String);

impl FieldPath {
    /// Wraps a dotted field path.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
