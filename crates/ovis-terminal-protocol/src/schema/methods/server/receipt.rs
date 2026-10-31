//! Inspectable operation state and retained method results.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::OperationEvidence;
use crate::errors::{ErrorData, OpId};
use crate::schema::ids::MethodName;
use crate::schema::scalar::{EventSeq, OpenEnum, QueuePosition};

/// State of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OpState {
    /// Admitted, not started.
    Accepted,
    /// In progress.
    Running,
    /// Durably committed.
    Committed,
    /// Failed without effects.
    Failed,
    /// Cancelled before commit.
    Cancelled,
    /// Effects may have begun; the outcome cannot be proven.
    OutcomeUnknown,
    /// Waiting in FIFO order for admission; never accepted, no effect has begun.
    Queued,
}

/// Parameters of `op.get` and `op.cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OpRefParams {
    /// Operation identity.
    pub op_id: OpId,
}

/// Inspectable operation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OpRecord {
    /// Operation identity.
    pub op_id: OpId,
    /// Method that created the operation.
    pub method: MethodName,
    /// Current state.
    pub state: OpenEnum<OpState>,
    /// Method result, once committed.
    pub result: Option<Value>,
    /// Structured error, once failed.
    pub error: Option<ErrorData>,
    /// Sequence of the latest state change.
    pub updated_seq: EventSeq,
    /// Supporting delivery, process and cancellation facts, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<OperationEvidence>,
    /// FIFO admission position, present only while the state is `queued`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_position: Option<QueuePosition>,
}

impl OpRecord {
    /// Creates a newly accepted operation before any external effect.
    pub const fn accepted(op_id: OpId, method: MethodName) -> Self {
        Self {
            op_id,
            method,
            state: OpenEnum::Known(OpState::Accepted),
            result: None,
            error: None,
            updated_seq: EventSeq::new(0),
            evidence: None,
            queued_position: None,
        }
    }

    /// Describes a request still waiting for admission; no durable record exists yet.
    pub const fn queued(op_id: OpId, method: MethodName, position: QueuePosition) -> Self {
        Self {
            op_id,
            method,
            state: OpenEnum::Known(OpState::Queued),
            result: None,
            error: None,
            updated_seq: EventSeq::new(0),
            evidence: None,
            queued_position: Some(position),
        }
    }
}
