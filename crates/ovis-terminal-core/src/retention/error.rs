//! Typed retention failures.

use ovis_terminal_store::StoreError;

use crate::events::EventError;

/// A floor read or sweep that cannot proceed; none is reported as an empty success.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RetentionError {
    /// A stored floor or page has a shape this build does not read.
    #[error("the stored retention record has an unsupported shape")]
    UnsupportedRecord,
    /// The floor record could not be encoded or decoded.
    #[error("the retention floor record could not be encoded or decoded")]
    Encode(#[from] serde_json::Error),
    /// The floor position cannot advance past the last evicted record.
    #[error("the retention position counter is exhausted")]
    Exhausted,
    /// The coordinator refused the sweep's unit; nothing was evicted.
    #[error(transparent)]
    Event(#[from] EventError),
    /// The store could not be read.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl RetentionError {
    /// Stable machine token.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnsupportedRecord => "invalid_record",
            Self::Encode(_) => "persistence_failed",
            Self::Exhausted => "counter_overflow",
            Self::Event(error) => error.kind(),
            Self::Store(error) => error.kind(),
        }
    }

    /// Recovery guidance.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::UnsupportedRecord => "open the store with the build that wrote it",
            Self::Encode(_) => "inspect the stored floor record; it was not replaced",
            Self::Exhausted => "start a new store; this class cannot advance further",
            Self::Event(error) => error.hint(),
            Self::Store(error) => error.hint(),
        }
    }
}
