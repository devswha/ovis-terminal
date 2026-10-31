//! First-message lookup and wait failures.

use thiserror::Error;

use crate::waits::WaitError;

/// A first-message request the launch book refuses.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LaunchError {
    /// No first message is queued under this launch.
    #[error("no first message is queued under this launch")]
    NotFound,
    /// The message belongs to another principal.
    #[error("the first message belongs to another principal")]
    Forbidden,
    /// Every retained message is still waiting for readiness.
    #[error("the first-message book is full of pending messages")]
    Capacity,
    /// A lock holder panicked.
    #[error("the first-message book is poisoned")]
    Poisoned,
    /// The readiness wait failed.
    #[error("the readiness wait failed")]
    Wait(#[source] WaitError),
}

impl LaunchError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Forbidden => "forbidden",
            Self::Capacity => "limit_exceeded",
            Self::Poisoned => "internal",
            Self::Wait(error) => error.kind(),
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotFound => "pass the op_id of a terminal.run that queued a first message",
            Self::Forbidden => "read a first message as the principal that launched it",
            Self::Capacity => "wait for pending first messages to settle before launching more",
            Self::Poisoned => "restart the runtime",
            Self::Wait(error) => error.hint(),
        }
    }
}
