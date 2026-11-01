//! Typed refusals of bridge sessions, deliveries and acknowledgements.

use ovis_terminal_protocol::errors::{
    BoundaryError, ErrorData, ErrorKind, ExceededBound, LimitName,
};
use ovis_terminal_protocol::schema::scalar::{Count, RecordVersion};

use crate::prompts::PromptError;
use crate::status::StatusError;

/// Why a bridge request was refused or could not be answered.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BridgeError {
    /// The token names no current program, or no current session of one.
    #[error("the token names no current program or bridge session")]
    Forbidden,
    /// The session already holds its bound of delivered, unacknowledged events.
    #[error("the session already holds its {} unacknowledged events", limit.get())]
    QueueFull {
        /// The session's queue bound.
        limit: Count,
    },
    /// No delivered, unacknowledged answer of this prompt is held by the session.
    #[error("no unacknowledged delivery of this prompt is held by the session")]
    NotDelivered,
    /// The acknowledgement names a version other than the delivered answer's.
    #[error("the acknowledged version is not the delivered version {}", current.get())]
    VersionConflict {
        /// The version the delivered answer committed.
        current: RecordVersion,
    },
    /// The prompt was not answered through an extension.
    #[error("the prompt has no answer delivered through an extension")]
    NotExtension,
    /// The committed prompt could not be read.
    #[error("the committed prompt could not be read")]
    Prompt(#[source] PromptError),
    /// The owning-process check failed.
    #[error("the owning-process check failed")]
    Owner(#[source] StatusError),
    /// The session registry lock was poisoned by a panic.
    #[error("the bridge session registry is unusable after a panic")]
    Poisoned,
}

impl BridgeError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::QueueFull { .. } => "limit_exceeded",
            Self::NotDelivered => "not_delivered",
            Self::VersionConflict { .. } => "version_conflict",
            Self::NotExtension => "not_found",
            Self::Prompt(error) => error.kind(),
            Self::Owner(error) => error.kind(),
            Self::Poisoned => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Forbidden => {
                "present the token handed to the terminal's current program, or attach again"
            }
            Self::QueueFull { .. } => "acknowledge delivered events before taking more",
            Self::NotDelivered => "acknowledge only events this session received and still holds",
            Self::VersionConflict { .. } => {
                "acknowledge the version carried by the delivered event"
            }
            Self::NotExtension => "read the prompt; only extension answers are delivered here",
            Self::Prompt(error) => error.hint(),
            Self::Owner(error) => error.hint(),
            Self::Poisoned => "restart the runtime; the bridge cannot be trusted after a panic",
        }
    }

    /// The wire error data of this refusal, with the bound or current version it names.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        match self {
            Self::Prompt(error) => return error.error_data(),
            Self::Owner(error) => return error.error_data(),
            _ => {}
        }
        let mut error = BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint());
        match self {
            Self::VersionConflict { current } => error = error.with_current_version(*current),
            Self::QueueFull { limit } => {
                let name = LimitName::new("bridge_queue".to_owned());
                error = error.with_bound(ExceededBound::new(name, *limit));
            }
            _ => {}
        }
        error.data().clone()
    }
}
