//! The screen boundary error.

use ovis_terminal_vt::{ContentRevision, OutputOffset};

use super::views::{CheckpointMark, RetainedRange, Unavailability};

/// Every rejection of screen ingestion or a terminal read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScreenError {
    /// Holder output named another terminal or incarnation.
    #[error("holder output belongs to another terminal or incarnation")]
    TargetMismatch,
    /// Holder output did not start where the screen stopped.
    #[error("holder output starts at byte {received}; the screen expects byte {expected}")]
    OutputGap {
        /// Next byte the screen expects.
        expected: OutputOffset,
        /// First byte the holder sent.
        received: OutputOffset,
    },
    /// The program already ended.
    #[error("the program ended at byte {final_offset}; no further output exists")]
    Ended {
        /// Offset of the final byte.
        final_offset: OutputOffset,
    },
    /// The requested raw output was evicted.
    #[error(
        "byte {requested} was evicted; raw output is retained from {} to {}",
        .retained.floor,
        .retained.head
    )]
    HistoryGap {
        /// Requested first byte.
        requested: OutputOffset,
        /// Bytes still retained.
        retained: RetainedRange,
        /// Newest checkpoint inside the retained range, when one exists.
        checkpoint: Option<CheckpointMark>,
    },
    /// The requested revision is neither live nor pinned.
    #[error(
        "content revision {requested} is no longer pinned; the screen is at revision {current}"
    )]
    StaleRevision {
        /// Revision the caller named.
        requested: ContentRevision,
        /// Current revision.
        current: ContentRevision,
    },
    /// The active screen keeps no history.
    #[error("history cannot be read at revision {revision}")]
    HistoryUnavailable {
        /// Why, and whether a transition ends it.
        unavailability: Unavailability,
        /// Revision the answer reflects.
        revision: ContentRevision,
        /// Offset the answer reflects.
        offset: OutputOffset,
    },
    /// The request does not match its closed schema.
    #[error("the request does not match its closed schema")]
    InvalidParams(#[source] serde_json::Error),
    /// A request field lies outside the snapshot it addresses.
    #[error("request field `{field}` lies outside the addressed snapshot")]
    InvalidRange {
        /// Offending field path.
        field: &'static str,
    },
    /// The bounds contradict each other.
    #[error("screen bounds are inconsistent: {reason}")]
    InvalidConfig {
        /// The violated relation.
        reason: &'static str,
    },
    /// A writer panicked while holding the screen state.
    #[error("the screen state lock was poisoned by a panicking writer")]
    Poisoned,
}

impl ScreenError {
    /// Stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TargetMismatch => "target_mismatch",
            Self::OutputGap { .. } => "output_gap",
            Self::Ended { .. } => "terminal_ended",
            Self::HistoryGap { .. } => "history_gap",
            Self::StaleRevision { .. } => "stale_revision",
            Self::HistoryUnavailable { .. } => "history_unavailable",
            Self::InvalidParams(_) | Self::InvalidRange { .. } => "invalid_params",
            Self::InvalidConfig { .. } => "invalid_config",
            Self::Poisoned => "screen_poisoned",
        }
    }

    /// Recovery hint for the caller.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::TargetMismatch => {
                "route holder output to the screen of its own terminal incarnation"
            }
            Self::OutputGap { .. } => {
                "recover the missing range from the holder tail or rebuild from a checkpoint"
            }
            Self::Ended { .. } => "read the final screen; the program produces no more output",
            Self::HistoryGap { .. } => {
                "resume from the retained floor, or start from the checkpoint and accept the gap"
            }
            Self::StaleRevision { .. } => "read again without a revision and page from the new pin",
            Self::HistoryUnavailable { .. } => {
                "wait for the named history_available transition unless recovery is permanent"
            }
            Self::InvalidParams(_) | Self::InvalidRange { .. } => {
                "send the request in its schema form with ranges inside the snapshot"
            }
            Self::InvalidConfig { .. } => {
                "raise retained output above checkpoint interval plus work budget"
            }
            Self::Poisoned => {
                "discard this screen and rebuild it from a checkpoint and retained output"
            }
        }
    }
}
