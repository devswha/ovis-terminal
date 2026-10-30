//! The VT engine boundary error.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::engine::units::ContentRevision;

/// Format identity a checkpoint declares and an engine accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CheckpointVersion {
    /// Format name.
    pub format: String,
    /// Format revision.
    pub version: u32,
    /// Emulator library the grid serialization belongs to.
    pub engine: String,
}

impl fmt::Display for CheckpointVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} v{} ({})", self.format, self.version, self.engine)
    }
}

/// Every failure the VT engine reports.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VtError {
    /// A zero-sized grid was requested.
    #[error("terminal geometry {columns}x{rows} is invalid; it needs at least 2 columns and 1 row")]
    InvalidGeometry {
        /// Requested columns.
        columns: u16,
        /// Requested rows.
        rows: u16,
    },
    /// The checkpoint was written by another format revision or emulator.
    #[error("checkpoint {found} is not readable by this engine, which reads {supported}")]
    CheckpointIncompatible {
        /// Identity the checkpoint declares.
        found: Box<CheckpointVersion>,
        /// Identity this engine reads.
        supported: Box<CheckpointVersion>,
    },
    /// The checkpoint bytes are not a decodable checkpoint.
    #[error("checkpoint bytes could not be decoded")]
    CheckpointMalformed(#[source] serde_json::Error),
    /// The checkpoint could not be serialized.
    #[error("checkpoint could not be encoded")]
    CheckpointEncoding(#[source] serde_json::Error),
    /// The checkpoint decodes but contradicts itself.
    #[error("checkpoint is inconsistent: {reason}")]
    CheckpointInconsistent {
        /// The violated invariant.
        reason: &'static str,
    },
    /// A revision-guarded read named a revision the terminal has moved past.
    #[error("content revision {expected} is stale; the terminal is at revision {current}")]
    RevisionMismatch {
        /// Revision the caller read against.
        expected: ContentRevision,
        /// Current revision.
        current: ContentRevision,
    },
}

impl VtError {
    /// Stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidGeometry { .. } => "invalid_geometry",
            Self::CheckpointIncompatible { .. } => "checkpoint_incompatible",
            Self::CheckpointMalformed(_) => "checkpoint_malformed",
            Self::CheckpointEncoding(_) => "checkpoint_encoding",
            Self::CheckpointInconsistent { .. } => "checkpoint_inconsistent",
            Self::RevisionMismatch { .. } => "revision_mismatch",
        }
    }

    /// Recovery hint for the caller.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidGeometry { .. } => "request at least two columns and one row",
            Self::CheckpointIncompatible { .. } => {
                "discard the checkpoint and rebuild the screen from retained output in a fresh engine"
            }
            Self::CheckpointMalformed(_) | Self::CheckpointInconsistent { .. } => {
                "quarantine the checkpoint and rebuild the screen from retained output"
            }
            Self::CheckpointEncoding(_) => "report the failure; the engine state is unchanged",
            Self::RevisionMismatch { .. } => {
                "read the terminal again and retry at its current revision"
            }
        }
    }

    pub(super) const fn revision_mismatch(
        expected: ContentRevision,
        current: ContentRevision,
    ) -> Self {
        Self::RevisionMismatch { expected, current }
    }
}
