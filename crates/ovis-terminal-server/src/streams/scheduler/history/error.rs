//! Why a history operation failed.

use ovis_terminal_vt::OutputOffset;
use thiserror::Error;

/// A history refusal or failure; a failed append leaves the store refusing appends.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HistoryError {
    /// The offset is below the retention floor; those bytes were evicted.
    #[error("offset {offset} was evicted; history starts at {floor}")]
    Evicted {
        /// Offset asked for.
        offset: OutputOffset,
        /// First offset still readable.
        floor: OutputOffset,
    },
    /// The offset is past the end of the recorded output.
    #[error("offset {offset} is past the recorded end {end}")]
    Beyond {
        /// Offset asked for.
        offset: OutputOffset,
        /// Offset after the last recorded byte.
        end: OutputOffset,
    },
    /// The segment size is zero or larger than the retention bound.
    #[error("a segment of {segment} bytes does not fit a retention bound of {retained}")]
    InvalidConfig {
        /// Bytes per segment.
        segment: u64,
        /// Bytes retained.
        retained: u64,
    },
    /// The segment files do not form one contiguous stream.
    #[error("segment {start} does not continue the stream that ends at {expected}")]
    Discontinuous {
        /// Start offset of the out-of-place segment.
        start: OutputOffset,
        /// Offset the previous segment ends at.
        expected: OutputOffset,
    },
    /// An earlier append failed; the history no longer records output.
    #[error("an earlier append failed with {0:?}; history stopped recording")]
    Faulted(std::io::ErrorKind),
    /// A file operation failed.
    #[error("history could not {operation}")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The file system error.
        #[source]
        source: std::io::Error,
    },
}

impl HistoryError {
    pub(super) fn io(operation: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io { operation, source }
    }

    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Evicted { .. } => "history_gap",
            Self::Beyond { .. } | Self::InvalidConfig { .. } => "invalid_params",
            Self::Discontinuous { .. } | Self::Faulted(_) | Self::Io { .. } => {
                "history_unavailable"
            }
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Evicted { .. } => "read from the floor, or take a checkpoint instead",
            Self::Beyond { .. } => "read at or below the recorded end",
            Self::InvalidConfig { .. } => "use a segment size between one byte and the bound",
            Self::Discontinuous { .. } | Self::Faulted(_) | Self::Io { .. } => {
                "check the store directory's disk space and permissions"
            }
        }
    }
}
