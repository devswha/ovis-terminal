//! Structured, serializable fields carried by a protocol error.

use serde::{Deserialize, Serialize};

use super::identity::{FieldPath, OpId, TargetId};
use super::kinds::ErrorKind;

/// Machine-readable context attached to a boundary failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ErrorData {
    /// Stable error category.
    pub kind: ErrorKind,
    /// Recovery guidance for a client.
    pub hint: String,
    /// Whether retrying the same request may succeed.
    pub retryable: bool,
    /// Path to the invalid input field, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_path: Option<FieldPath>,
    /// Stable target identity, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetId>,
    /// Operation identity when effects may have begun.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_id: Option<OpId>,
}

impl ErrorData {
    pub(super) fn new(kind: ErrorKind, retryable: bool) -> Self {
        Self {
            kind,
            hint: kind.hint().to_owned(),
            retryable,
            field_path: None,
            target: None,
            op_id: None,
        }
    }

    pub(super) fn with_field_path(mut self, field_path: Option<&str>) -> Self {
        self.field_path = field_path.map(|path| FieldPath::new(path.to_owned()));
        self
    }
}
