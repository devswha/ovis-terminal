//! Typed refusals of record appends and record reads.

use ovis_terminal_core::events::EventError;
use ovis_terminal_protocol::domain::conversation_records::Unavailable;
use ovis_terminal_protocol::domain::session_binding::HistoryHead;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};

use crate::status::StatusError;

/// How a reported record breaks the typed record contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Violation {
    /// An append carries no record and no turn metadata, or a record has no block.
    Empty,
    /// A field, block list or batch exceeds its bound.
    Bounds,
    /// The record identity is already recorded.
    DuplicateRecord,
    /// The tool call identity is already recorded.
    DuplicateCall,
    /// The tool result answers no recorded call.
    DanglingCall,
    /// The block type is not one the record's role may carry.
    BlockRole,
    /// A phase is reported for a record that is not assistant text.
    PhaseRole,
    /// Turn metadata is reported while the status root is not working.
    NoRunningTurn,
}

impl Violation {
    const fn hint(self) -> &'static str {
        match self {
            Self::Empty => "send at least one record with at least one block, or turn metadata",
            Self::Bounds => {
                "keep record ids within 128 bytes, call ids, names and turn labels within 256, texts within 64 KiB, records within 256 KiB and 32 blocks, appends within 64 records"
            }
            Self::DuplicateRecord => "give every record a new record_id",
            Self::DuplicateCall => "give every tool call a new call_id",
            Self::DanglingCall => "append the record with the tool call before its result",
            Self::BlockRole => {
                "user records carry text and tool results, assistant records text and tool calls, tool records tool results"
            }
            Self::PhaseRole => "report a phase only on assistant records",
            Self::NoRunningTurn => "report turn metadata while the status root is working",
        }
    }
}

/// Why an append or a read was refused or could not be persisted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecordsError {
    /// The token does not belong to the target terminal's current program.
    #[error("the process token does not own the target terminal's current program")]
    NotOwner,
    /// The current program registered no agent.
    #[error("the current program has no registered agent to report records for")]
    Unregistered,
    /// A reported field breaks the typed record contract.
    #[error("the reported field {field} breaks the record contract ({violation:?})")]
    InvalidRecords {
        /// Path of the offending field below `params`.
        field: String,
        /// The broken rule.
        violation: Violation,
    },
    /// The agent's records would exceed their retained bound.
    #[error("the agent's records would exceed their retained bound")]
    TooLarge,
    /// No structured conversation exists for the terminal.
    #[error("no structured conversation is recorded for the terminal")]
    Unavailable(Box<Unavailable>),
    /// The cursor names another identity or revision than the records have now.
    #[error("the records moved since the cursor was issued")]
    HistoryChanged {
        /// Identity and revision the records have now.
        current: HistoryHead,
    },
    /// The cursor was not issued for these records.
    #[error("the cursor does not describe a position of the records")]
    CursorExpired,
    /// The agent status row refused or failed.
    #[error("the agent status record refused the request")]
    Status(#[source] StatusError),
    /// The record log could not be read or written.
    #[error("the record log could not be persisted")]
    Event(#[from] EventError),
    /// The record log could not be encoded or decoded.
    #[error("the record log could not be encoded or decoded")]
    Encode(#[from] serde_json::Error),
}

impl From<StatusError> for RecordsError {
    fn from(error: StatusError) -> Self {
        match error {
            StatusError::NotOwner => Self::NotOwner,
            other => Self::Status(other),
        }
    }
}

impl RecordsError {
    /// A contract violation at `field`.
    pub(super) const fn invalid(field: String, violation: Violation) -> Self {
        Self::InvalidRecords { field, violation }
    }

    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotOwner | Self::Unregistered => "not_owner",
            Self::InvalidRecords { .. } => "invalid_records",
            Self::TooLarge => "too_large",
            Self::Unavailable(_) => "conversation_unavailable",
            Self::HistoryChanged { .. } => "history_changed",
            Self::CursorExpired => "cursor_expired",
            Self::Status(error) => error.kind(),
            Self::Event(_) | Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotOwner => "report with the token handed to the terminal's current program",
            Self::Unregistered => "register the agent with agent.register before reporting records",
            Self::InvalidRecords { violation, .. } => violation.hint(),
            Self::TooLarge => "an agent keeps at most 10000 records and 8 MiB of them",
            Self::Unavailable(_) => {
                "the agent reports no structured records; show its terminal output as terminal output"
            }
            Self::HistoryChanged { .. } => {
                "read again with the current revision; earlier positions still hold after an append"
            }
            Self::CursorExpired => "restart from the first page of the current revision",
            Self::Status(error) => error.hint(),
            Self::Event(_) | Self::Encode(_) => "inspect the operation before retrying",
        }
    }

    /// The wire error data of this refusal.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        let mut error = BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint());
        match self {
            Self::InvalidRecords { field, .. } => {
                error = error.with_field_path(&format!("params.{field}"));
            }
            Self::Unavailable(unavailable) => {
                error = error.with_unavailable(unavailable.as_ref().clone());
            }
            Self::HistoryChanged { current } => error = error.with_history(current.clone()),
            _ => {}
        }
        error.data().clone()
    }
}
