//! Typed refusals of status reports and agent registrations.

use ovis_terminal_core::events::EventError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};
use ovis_terminal_protocol::schema::scalar::ReportSeq;

/// Why a status or agent request was refused or could not be persisted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StatusError {
    /// The producer sequence does not exceed the accepted one.
    #[error("the report sequence is not above the accepted sequence {}", current.get())]
    StaleSeq {
        /// Highest accepted producer sequence.
        current: ReportSeq,
    },
    /// The token does not belong to the target terminal's current program.
    #[error("the process token does not own the target terminal's current program")]
    NotOwner,
    /// The update names a program incarnation that is not the terminal's current one.
    #[error("the update comes from a program incarnation that is no longer current")]
    StaleIncarnation,
    /// A record field is outside its accepted form.
    #[error("the status record field {field} is outside its accepted form")]
    InvalidRecord {
        /// The offending field.
        field: &'static str,
    },
    /// The agent label is empty or too long.
    #[error("the agent label must hold 1 to 256 bytes")]
    InvalidAgent,
    /// This program incarnation already registered an agent.
    #[error("the program incarnation already has a registered agent")]
    AgentConflict,
    /// The token registry lock was poisoned by a panic.
    #[error("the process token registry is unusable after a panic")]
    Poisoned,
    /// The committed status record could not be read or written.
    #[error("the status record could not be persisted")]
    Event(#[from] EventError),
    /// The committed status record could not be encoded or decoded.
    #[error("the status record could not be encoded or decoded")]
    Encode(#[from] serde_json::Error),
}

impl StatusError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::StaleSeq { .. } => "stale_seq",
            Self::NotOwner => "not_owner",
            Self::StaleIncarnation => "stale_incarnation",
            Self::InvalidRecord { .. } => "invalid_record",
            Self::InvalidAgent => "invalid_agent",
            Self::AgentConflict => "agent_conflict",
            Self::Poisoned | Self::Event(_) | Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::StaleSeq { .. } => "report again with a sequence above current_seq",
            Self::NotOwner => "report with the token handed to the terminal's current program",
            Self::StaleIncarnation => "only the terminal's current program changes its status",
            Self::InvalidRecord { .. } => {
                "send a known state, progress 0-100 and fields within their bounds"
            }
            Self::InvalidAgent => "register a label of 1 to 256 bytes",
            Self::AgentConflict => "read agent.get; the incarnation keeps its first agent",
            Self::Poisoned | Self::Event(_) | Self::Encode(_) => {
                "inspect the operation before retrying"
            }
        }
    }

    /// Whether the request was refused by the domain rather than failing internally.
    #[must_use]
    pub const fn is_rejection(&self) -> bool {
        !matches!(self, Self::Poisoned | Self::Event(_) | Self::Encode(_))
    }

    /// The wire error data of this refusal, with `current_seq` for a stale sequence.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        let mut error = BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint());
        if let Self::StaleSeq { current } = self {
            error = error.with_current_seq(*current);
        }
        error.data().clone()
    }
}
