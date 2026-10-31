//! Why a shard refused output.

use thiserror::Error;

/// Output the shard refused; nothing of it was queued.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShardError {
    /// Queuing the chunk would pass the pending cap.
    #[error("{offered} more bytes would pass the {cap}-byte cap; {pending} bytes already wait")]
    Full {
        /// Bytes offered.
        offered: usize,
        /// Bytes already waiting.
        pending: usize,
        /// The pending cap.
        cap: usize,
    },
}

impl ShardError {
    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Full { .. } => "backpressure",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Full { .. } => "run parse turns until pending output drains, then offer it again",
        }
    }
}
