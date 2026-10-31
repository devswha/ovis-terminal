//! Typed refusals of session bindings and conversation reads.

use ovis_terminal_core::events::EventError;
use ovis_terminal_protocol::domain::session_binding::HistoryHead;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};

use crate::status::StatusError;

/// Why a binding or a conversation read was refused or could not be persisted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BindingError {
    /// The token does not belong to the target terminal's current program.
    #[error("the process token does not own the target terminal's current program")]
    NotOwner,
    /// The current program registered no agent.
    #[error("the current program has no registered agent to bind")]
    Unregistered,
    /// A session field is outside its accepted form or contradicts the binding.
    #[error("the session field {field} is not accepted")]
    InvalidSession {
        /// The offending request field.
        field: &'static str,
    },
    /// A transcript path resolves outside every authorized store.
    #[error("the transcript path resolves outside the authorized stores")]
    OutsideStores,
    /// The cursor names another identity or revision than the history has now.
    #[error("the history moved since the cursor was issued")]
    HistoryChanged {
        /// Identity and revision the history has now.
        current: HistoryHead,
    },
    /// The cursor position lies beyond its chain, or its token was not issued here.
    #[error("the cursor no longer describes a position of its history")]
    CursorExpired,
    /// A record the request names does not exist at the current revision.
    #[error("the named record is not part of the history")]
    UnknownRecord,
    /// The terminal's agent has no history bound.
    #[error("no conversation history is bound to the terminal's agent")]
    Unbound,
    /// The bound session has no readable transcript.
    #[error("the bound session is not backed by a readable transcript")]
    NoTranscript,
    /// The transcript cannot be opened or read.
    #[error("the transcript cannot be read")]
    SourceMissing {
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The transcript exceeds the readable bound.
    #[error("the transcript exceeds the readable size bound")]
    TooLarge,
    /// A complete transcript line is not a history record.
    #[error("transcript line {line} is not a history record")]
    Malformed {
        /// One-based line number.
        line: usize,
    },
    /// The agent status row refused or failed.
    #[error("the agent status record refused the binding")]
    Status(#[source] StatusError),
    /// The committed history record could not be read or written.
    #[error("the history record could not be persisted")]
    Event(#[from] EventError),
    /// The committed history record could not be encoded or decoded.
    #[error("the history record could not be encoded or decoded")]
    Encode(#[from] serde_json::Error),
}

impl From<StatusError> for BindingError {
    fn from(error: StatusError) -> Self {
        match error {
            StatusError::NotOwner => Self::NotOwner,
            other => Self::Status(other),
        }
    }
}

impl BindingError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotOwner | Self::Unregistered => "not_owner",
            Self::InvalidSession { .. } | Self::OutsideStores => "invalid_session",
            Self::HistoryChanged { .. } => "history_changed",
            Self::CursorExpired => "cursor_expired",
            Self::UnknownRecord => "not_found",
            Self::Unbound
            | Self::NoTranscript
            | Self::SourceMissing { .. }
            | Self::TooLarge
            | Self::Malformed { .. } => "conversation_unavailable",
            Self::Status(error) => error.kind(),
            Self::Event(_) | Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotOwner => "bind with the token handed to the terminal's current program",
            Self::Unregistered => "register the agent with agent.register before binding a session",
            Self::InvalidSession { .. } => {
                "send kind path or id with a bounded value; once bound, name the reason, and a head only with reason branch"
            }
            Self::OutsideStores => "report a transcript inside an authorized provider store",
            Self::HistoryChanged { .. } => {
                "read again with the current history identity and revision; after an append the same position still applies"
            }
            Self::CursorExpired => "restart from the first page of the current revision",
            Self::UnknownRecord => "address a record listed in the current revision",
            Self::Unbound => "bind a session with agent.update_session first",
            Self::NoTranscript => "bind a session of kind path to read its conversation",
            Self::SourceMissing { .. } => {
                "restore the transcript or bind the session that replaced it"
            }
            Self::TooLarge => "the transcript exceeds 8 MiB; bind a smaller session",
            Self::Malformed { .. } => "repair the transcript line or bind another session",
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
            Self::InvalidSession { field } => {
                error = error.with_field_path(&format!("params.{field}"));
            }
            Self::HistoryChanged { current } => error = error.with_history(current.clone()),
            _ => {}
        }
        error.data().clone()
    }
}
