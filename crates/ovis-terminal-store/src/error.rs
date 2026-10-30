//! The store's boundary error: one stable kind and recovery hint per variant.

use std::path::PathBuf;

use crate::evidence::RecoveryEvidence;
use crate::ids::{OpId, ResourceKey, Version};

/// Every failure the store reports; no failure is turned into an empty success.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The database is damaged; its files were moved into quarantine.
    #[error("store database is corrupted; its material was quarantined at {}", .evidence.location.display())]
    Corrupted {
        /// The durable evidence of the quarantine.
        evidence: Box<RecoveryEvidence>,
    },
    /// The database was written by a newer schema; it was left in place and copied.
    #[error("store schema is newer than this build; a copy was preserved at {}", .evidence.location.display())]
    Unsupported {
        /// The durable evidence of the preserved copy.
        evidence: Box<RecoveryEvidence>,
    },
    /// Rebuild was asked for although no quarantine moved the database away.
    #[error("nothing to rebuild under {}", .root.display())]
    NothingToRebuild {
        /// The store root.
        root: PathBuf,
    },
    /// A compare-and-swap expected a version the resource is not at.
    #[error("{key} is at version {current}, not {expected}")]
    Conflict {
        /// The resource.
        key: ResourceKey,
        /// The version the mutation expected.
        expected: Version,
        /// The version the resource is at.
        current: Version,
    },
    /// The operation id already has a committed receipt.
    #[error("operation {op} is already committed")]
    DuplicateOperation {
        /// The reused operation id.
        op: OpId,
    },
    /// A payload exceeds the advertised size limit.
    #[error("a payload of {actual} bytes exceeds the {limit}-byte limit")]
    TooLarge {
        /// The advertised limit in bytes.
        limit: usize,
        /// The rejected payload size in bytes.
        actual: usize,
    },
    /// The bounded writer queue is full.
    #[error("the writer queue is full ({capacity} pending mutations)")]
    Backpressure {
        /// The advertised queue capacity.
        capacity: usize,
    },
    /// The writer stopped before answering.
    #[error("the store writer has stopped")]
    WriterStopped,
    /// The writer thread panicked.
    #[error("the store writer thread panicked")]
    WriterPanicked,
    /// A sequence or version counter is exhausted.
    #[error("the {counter} counter is exhausted")]
    CounterOverflow {
        /// Which counter.
        counter: &'static str,
    },
    /// SQLite refused to switch the database to write-ahead logging.
    #[error("the database stayed in journal mode {mode} instead of WAL")]
    WalUnavailable {
        /// The journal mode SQLite kept.
        mode: String,
    },
    /// SQLite failed.
    #[error("sqlite operation failed")]
    Sqlite(#[from] rusqlite::Error),
    /// A file operation on the store root failed.
    #[error("cannot {action} {}", .path.display())]
    Io {
        /// The attempted operation.
        action: &'static str,
        /// The path it was attempted on.
        path: PathBuf,
        /// The operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// Recovery evidence could not be encoded or decoded.
    #[error("cannot encode or decode recovery evidence at {}", .path.display())]
    Evidence {
        /// The evidence file.
        path: PathBuf,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
}

impl StoreError {
    /// The stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Corrupted { .. } => "store_corrupted",
            Self::Unsupported { .. } => "store_unsupported",
            Self::NothingToRebuild { .. } => "nothing_to_rebuild",
            Self::Conflict { .. } => "conflict",
            Self::DuplicateOperation { .. } => "duplicate_operation",
            Self::TooLarge { .. } => "too_large",
            Self::Backpressure { .. } => "backpressure",
            Self::WriterStopped => "writer_stopped",
            Self::WriterPanicked => "writer_panicked",
            Self::CounterOverflow { .. } => "counter_overflow",
            Self::WalUnavailable { .. } => "wal_unavailable",
            Self::Sqlite(_) | Self::Io { .. } | Self::Evidence { .. } => "persistence_failed",
        }
    }

    /// The recovery hint for this kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Corrupted { .. } => {
                "inspect the quarantined database and WAL, then rebuild_after_quarantine starts an empty store that keeps this evidence"
            }
            Self::Unsupported { .. } => {
                "open the store with the newer build that wrote it; the original stays in place and a copy is in quarantine"
            }
            Self::NothingToRebuild { .. } => {
                "open the store normally; rebuild applies only after a quarantine moved the database"
            }
            Self::Conflict { .. } => {
                "read the current version and retry the compare-and-swap with it"
            }
            Self::DuplicateOperation { .. } => {
                "read the retained receipt of this operation id instead of committing again"
            }
            Self::TooLarge { .. } => {
                "reduce the payload below the advertised max_value_bytes limit"
            }
            Self::Backpressure { .. } => "retry after pending commits complete",
            Self::WriterStopped | Self::WriterPanicked => {
                "reopen the store; receipts show which in-flight mutations committed"
            }
            Self::CounterOverflow { .. } => {
                "start a new store; this one cannot allocate further sequence numbers"
            }
            Self::WalUnavailable { .. } => {
                "place the store root on a local file system that supports shared memory"
            }
            Self::Sqlite(_) | Self::Io { .. } | Self::Evidence { .. } => {
                "check free space and permissions of the store root; the mutation was not acknowledged"
            }
        }
    }

    /// The recovery evidence carried by `store_corrupted` and `store_unsupported`.
    pub fn recovery_evidence(&self) -> Option<&RecoveryEvidence> {
        match self {
            Self::Corrupted { evidence } | Self::Unsupported { evidence } => Some(evidence),
            Self::NothingToRebuild { .. }
            | Self::Conflict { .. }
            | Self::DuplicateOperation { .. }
            | Self::TooLarge { .. }
            | Self::Backpressure { .. }
            | Self::WriterStopped
            | Self::WriterPanicked
            | Self::CounterOverflow { .. }
            | Self::WalUnavailable { .. }
            | Self::Sqlite(_)
            | Self::Io { .. }
            | Self::Evidence { .. } => None,
        }
    }

    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
    ) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io {
            action,
            path,
            source,
        }
    }
}
