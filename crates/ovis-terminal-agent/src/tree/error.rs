//! Typed refusals of child links, exits and acknowledgements.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};

use crate::prompts::PromptError;
use crate::status::StatusError;

/// Why a tree request was refused or could not be persisted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TreeError {
    /// The parent terminal has no program this runtime knows.
    #[error("the parent terminal runs no known program")]
    ParentNotFound,
    /// The named parent incarnation already finished.
    #[error("the named parent incarnation already finished")]
    ParentFinished,
    /// The named parent incarnation is not the terminal's current program.
    #[error("the named parent incarnation is not the terminal's current program")]
    StaleTarget,
    /// The parent incarnation already has the most running children it keeps.
    #[error("the parent incarnation already has its limit of running children")]
    LimitExceeded,
    /// The program's finish was already recorded; nothing changed.
    #[error("this program incarnation's finish was already recorded")]
    AlreadyFinished,
    /// Everything named was already acknowledged; nothing changed.
    #[error("everything named was already acknowledged")]
    AlreadyAcknowledged,
    /// Reading the parent's or child's declared status failed.
    #[error("the declared status could not be read")]
    Status(#[source] StatusError),
    /// Expiring the ended child's prompts failed.
    #[error("the ended child's prompts could not be expired")]
    Prompt(#[source] PromptError),
    /// A registry lock was poisoned by a panic.
    #[error("the tree registry is unusable after a panic")]
    Poisoned,
    /// The tree record could not be read or written.
    #[error("the tree record could not be persisted")]
    Event(#[from] EventError),
    /// The tree record could not be encoded or decoded.
    #[error("the tree record could not be encoded or decoded")]
    Encode(#[from] serde_json::Error),
}

impl From<StatusError> for TreeError {
    fn from(error: StatusError) -> Self {
        match error {
            StatusError::Poisoned => Self::Poisoned,
            StatusError::Event(source) => Self::Event(source),
            StatusError::Encode(source) => Self::Encode(source),
            refusal => Self::Status(refusal),
        }
    }
}

impl From<PromptError> for TreeError {
    fn from(error: PromptError) -> Self {
        match error {
            PromptError::Poisoned => Self::Poisoned,
            PromptError::Event(source) => Self::Event(source),
            PromptError::Encode(source) => Self::Encode(source),
            refusal => Self::Prompt(refusal),
        }
    }
}

impl TreeError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ParentNotFound => "parent_not_found",
            Self::ParentFinished => "parent_finished",
            Self::StaleTarget => "stale_target",
            Self::LimitExceeded => "limit_exceeded",
            Self::AlreadyFinished => "already_finished",
            Self::AlreadyAcknowledged => "already_acknowledged",
            Self::Status(error) => error.kind(),
            Self::Prompt(error) => error.kind(),
            Self::Poisoned | Self::Event(_) | Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::ParentNotFound => "name a parent terminal that runs a program",
            Self::ParentFinished => {
                "the parent incarnation ended; launch without a parent or under a live one"
            }
            Self::StaleTarget => {
                "read the parent terminal's current incarnation and name that one if it applies"
            }
            Self::LimitExceeded => "wait for a child to end before linking another",
            Self::AlreadyFinished => "the finish is recorded; read the children instead",
            Self::AlreadyAcknowledged => "nothing remains to acknowledge",
            Self::Status(error) => error.hint(),
            Self::Prompt(error) => error.hint(),
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

    /// Settles a tree outcome inside an operation unit: refusals fail the operation with
    /// their typed data, internal failures stay errors.
    ///
    /// # Errors
    ///
    /// Returns the store, decoding or lock failure.
    pub fn settle<D>(
        outcome: Result<Completion<D>, Self>,
    ) -> Result<Completion<D>, OperationError> {
        match outcome {
            Ok(completion) => Ok(completion),
            Err(Self::Encode(error)) => Err(error.into()),
            Err(Self::Event(error)) => Err(error.into()),
            Err(Self::Poisoned) => Err(EventError::Poisoned.into()),
            Err(refusal) => Ok(Completion::failed(refusal.error_data())),
        }
    }

    /// The wire error data of this refusal.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint())
            .data()
            .clone()
    }
}
