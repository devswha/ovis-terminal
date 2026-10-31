//! Structured, serializable fields carried by a protocol error.

use serde::{Deserialize, Serialize};

use super::bound::ExceededBound;
use super::identity::{FieldPath, OpId, TargetId};
use super::kinds::ErrorKind;
use crate::domain::session_binding::HistoryHead;
use crate::schema::scalar::{DurationMs, QueuePosition, ReportSeq};

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
    /// Dispatch budget, present when a request reaches its deadline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<DurationMs>,
    /// Monotonic elapsed time when the deadline failure was produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<DurationMs>,
    /// Current producer status sequence, present on a `stale_seq` rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_seq: Option<ReportSeq>,
    /// Fully qualified alternatives for an ambiguous target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<super::TargetCandidate>,
    /// Every resource a refusal would touch or a partial effect already committed.
    #[serde(default, skip_serializing_if = "<[TargetId]>::is_empty")]
    pub affected: Box<[TargetId]>,
    /// FIFO admission position of an operation that is still queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_position: Option<QueuePosition>,
    /// The advertised limit a `limit_exceeded` refusal reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound: Option<ExceededBound>,
    /// Current history identity and revision, present on a `history_changed` refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Box<HistoryHead>>,
}

impl ErrorData {
    pub(super) fn new(kind: ErrorKind, retryable: bool) -> Self {
        Self {
            hint: kind.hint().to_owned(),
            kind,
            retryable,
            field_path: None,
            target: None,
            op_id: None,
            deadline_ms: None,
            elapsed_ms: None,
            current_seq: None,
            candidates: Vec::new(),
            affected: Box::default(),
            queued_position: None,
            bound: None,
            history: None,
        }
    }

    pub(super) fn with_field_path(mut self, field_path: Option<&str>) -> Self {
        self.field_path = field_path.map(|path| FieldPath::new(path.to_owned()));
        self
    }
}
