//! Typed refusals of prompt publication, answers, closes and blocked input.

use ovis_terminal_core::events::EventError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};
use ovis_terminal_protocol::schema::ids::PromptId;
use ovis_terminal_protocol::schema::scalar::RecordVersion;

use super::KeyInputError;
use crate::status::StatusError;

/// Why a prompt request was refused or could not be persisted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PromptError {
    /// The process token does not own the terminal's current program or this prompt.
    #[error("the process token does not own this prompt's program")]
    NotOwner,
    /// A prompt definition field is outside its accepted form.
    #[error("the prompt field {field} is outside its accepted form")]
    InvalidPrompt {
        /// The offending field.
        field: &'static str,
    },
    /// The terminal already has the most open prompts it keeps.
    #[error("the terminal already holds its limit of open prompts")]
    LimitExceeded,
    /// No prompt of this identity exists on the resolved terminal.
    #[error("no prompt of this identity exists")]
    NotFound,
    /// The answer names a version that is no longer current.
    #[error("the answered version is not the prompt's current version {}", current.get())]
    VersionConflict {
        /// The prompt's current version.
        current: RecordVersion,
    },
    /// The prompt was answered, dismissed or expired.
    #[error("the prompt no longer accepts answers")]
    PromptClosed,
    /// The owning program closes a prompt that is already closed.
    #[error("the prompt is already closed")]
    AlreadyClosed,
    /// The answer does not match the prompt's choices, fields or text bound.
    #[error("the answer field {field} does not match the prompt")]
    AnswerInvalid {
        /// The offending answer field.
        field: &'static str,
    },
    /// The prompt's mechanism cannot deliver this answer.
    #[error("the prompt's mechanism cannot deliver this answer")]
    DeliveryUnsupported,
    /// Ordinary input is refused while the program waits on these prompts.
    #[error("the program is blocked on {} open prompts", prompt_ids.len())]
    Blocked {
        /// Open prompts; answer one of them instead.
        prompt_ids: Vec<PromptId>,
    },
    /// The terminal's input path refused the typed answer.
    #[error("the typed answer was not admitted to the terminal's input")]
    Input(#[source] KeyInputError),
    /// The owning-process check failed.
    #[error("the owning-process check failed")]
    Owner(#[source] StatusError),
    /// A registry lock was poisoned by a panic.
    #[error("the prompt registry is unusable after a panic")]
    Poisoned,
    /// The committed prompt record could not be read or written.
    #[error("the prompt record could not be persisted")]
    Event(#[from] EventError),
    /// The committed prompt record could not be encoded or decoded.
    #[error("the prompt record could not be encoded or decoded")]
    Encode(#[from] serde_json::Error),
}

impl From<StatusError> for PromptError {
    fn from(error: StatusError) -> Self {
        match error {
            StatusError::Poisoned => Self::Poisoned,
            StatusError::Event(source) => Self::Event(source),
            StatusError::Encode(source) => Self::Encode(source),
            StatusError::NotOwner => Self::NotOwner,
            refusal => Self::Owner(refusal),
        }
    }
}

impl PromptError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotOwner => "not_owner",
            Self::InvalidPrompt { .. } => "invalid_prompt",
            Self::LimitExceeded => "limit_exceeded",
            Self::NotFound => "not_found",
            Self::VersionConflict { .. } => "version_conflict",
            Self::PromptClosed => "prompt_closed",
            Self::AlreadyClosed => "already_closed",
            Self::AnswerInvalid { .. } => "answer_invalid",
            Self::DeliveryUnsupported => "delivery_unsupported",
            Self::Blocked { .. } => "blocked",
            Self::Input(error) => error.kind(),
            Self::Owner(error) => error.kind(),
            Self::Poisoned | Self::Event(_) | Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotOwner => "only the program holding the prompt's process token changes it",
            Self::InvalidPrompt { .. } => {
                "send text, choices or fields within their bounds and an open prompt_id you own"
            }
            Self::LimitExceeded => "close or answer an open prompt before opening another",
            Self::NotFound => "list the terminal's prompts with prompt.list",
            Self::VersionConflict { .. } => {
                "read the prompt again and answer its current_version only if it still applies"
            }
            Self::PromptClosed => "read the prompt; a closed prompt takes no answer",
            Self::AlreadyClosed => "the prompt is closed; nothing remains to close",
            Self::AnswerInvalid { .. } => {
                "answer with a declared choice id, the declared fields, or bounded text"
            }
            Self::DeliveryUnsupported => {
                "answer keys prompts with text; this runtime cannot deliver this answer"
            }
            Self::Blocked { .. } => {
                "answer one of prompt_ids with prompt.answer, or use input.raw with its own grant"
            }
            Self::Input(error) => error.hint(),
            Self::Owner(error) => error.hint(),
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

    /// The wire error data of this refusal with its current version, field or prompts.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        let mut error = BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
            .with_hint(self.hint());
        match self {
            Self::VersionConflict { current } => error = error.with_current_version(*current),
            Self::Blocked { prompt_ids } => error = error.with_prompt_ids(prompt_ids.clone()),
            Self::InvalidPrompt { field } | Self::AnswerInvalid { field } => {
                error = error.with_field_path(field);
            }
            _ => {}
        }
        error.data().clone()
    }
}
