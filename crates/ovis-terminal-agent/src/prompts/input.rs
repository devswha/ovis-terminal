//! The terminal input path a `keys` answer is admitted to.
//!
//! Admission runs inside the answer's commit unit, after every prompt check, so a refused
//! or unauthorized answer never reaches it and no bytes are queued for it.

use std::fmt::Debug;

use ovis_terminal_protocol::schema::ids::LeaseId;

use crate::status::ProcessOwner;

/// Literal answer text for one program, and the interact lease the caller presented.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AnswerKeys {
    /// The program the prompt belongs to.
    pub owner: ProcessOwner,
    /// Interact lease presented with the answer.
    pub lease: Option<LeaseId>,
    /// Literal text; never interpreted as key names or control requests.
    pub text: String,
}

/// Why the input path did not admit a typed answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KeyInputError {
    /// An interact lease exists on the terminal and the caller does not hold it.
    #[error("an interact lease exists and the answer does not hold it")]
    NoLease,
    /// The presented lease was revoked.
    #[error("the presented interact lease was revoked")]
    LeaseRevoked,
    /// The program the prompt belongs to is no longer the terminal's current one.
    #[error("the prompt's program is no longer the terminal's current program")]
    ProcessReplaced,
    /// The input queue cannot take more bytes.
    #[error("the terminal's input queue is full")]
    InputOverflow,
}

impl KeyInputError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NoLease => "no_lease",
            Self::LeaseRevoked => "lease_revoked",
            Self::ProcessReplaced => "process_replaced",
            Self::InputOverflow => "input_overflow",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NoLease => "acquire the interact lease and answer with its lease_id",
            Self::LeaseRevoked => "acquire the interact lease again before answering",
            Self::ProcessReplaced => "read the terminal's prompts; this prompt has expired",
            Self::InputOverflow => "retry after the queued input drains",
        }
    }
}

/// Queues literal text on a terminal's ordered input path.
pub trait KeyInput: Debug + Send + Sync + 'static {
    /// Admits `keys` for its program, or refuses without queueing any byte.
    ///
    /// # Errors
    ///
    /// Returns the typed reason the text was not queued.
    fn admit(&self, keys: &AnswerKeys) -> Result<(), KeyInputError>;
}
