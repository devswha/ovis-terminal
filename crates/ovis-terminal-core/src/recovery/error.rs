//! The crash-recovery boundary error.

use std::path::PathBuf;

/// Every failure of crash recovery, checkpoint persistence or the incident journal.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecoveryError {
    /// The operating system boot identity could not be read.
    #[error("the operating system boot identity could not be read")]
    BootEvidence(#[source] std::io::Error),
    /// The recorded boot stamp could not be written or read.
    #[error("the boot stamp could not be {operation}")]
    BootStampIo {
        /// What failed.
        operation: &'static str,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The recorded boot stamp is not of the published shape; it is kept.
    #[error("the boot stamp on disk does not have the published shape")]
    BootStampMalformed(#[source] serde_json::Error),
    /// A checkpoint could not be written or read.
    #[error("the screen checkpoint could not be {operation}")]
    CheckpointIo {
        /// What failed.
        operation: &'static str,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A checkpoint failed its checksum or did not decode; it was moved to quarantine.
    #[error("the screen checkpoint is damaged and was kept at {}", .quarantine.display())]
    CheckpointCorrupted {
        /// Where the damaged file was kept.
        quarantine: PathBuf,
    },
    /// A checkpoint names a format this build cannot read; it was moved to quarantine.
    #[error("the screen checkpoint has an unsupported format and was kept at {}", .quarantine.display())]
    CheckpointUnsupported {
        /// Where the file was kept.
        quarantine: PathBuf,
    },
    /// A checkpoint older than the stored one was offered; the stored one stays.
    #[error("checkpoint at offset {offered} is older than the stored one at {stored}")]
    CheckpointStale {
        /// Offset of the stored checkpoint.
        stored: u64,
        /// Offset of the refused checkpoint.
        offered: u64,
    },
    /// A checkpoint belongs to another terminal incarnation.
    #[error("the checkpoint belongs to another terminal incarnation")]
    CheckpointTarget,
    /// The checkpoint contradicts the engine that must import it.
    #[error("the checkpoint could not be imported")]
    Engine(#[from] ovis_terminal_vt::VtError),
    /// A holder group link failed at `stage`.
    #[error("the holder link failed while trying to {stage}")]
    HolderLink {
        /// What was attempted.
        stage: &'static str,
        /// The link failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The holder answered with a message the recovery step does not accept.
    #[error("the holder sent another message than {expected}")]
    HolderProtocol {
        /// What was expected.
        expected: &'static str,
    },
    /// The incident journal could not be written or read.
    #[error("the incident journal could not be {operation}")]
    JournalIo {
        /// What failed.
        operation: &'static str,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A journal line is not an incident; the journal is kept as it is.
    #[error("incident journal line {line} is not an incident record")]
    JournalCorrupt {
        /// One-based line number.
        line: usize,
        /// The decode failure.
        #[source]
        source: serde_json::Error,
    },
    /// The state database refused to open; its evidence is kept by the store.
    #[error("the state database could not be opened as it is")]
    Store(#[from] ovis_terminal_store::StoreError),
    /// The wall clock lies before the Unix epoch or past a millisecond timestamp.
    #[error("the wall clock is not representable as a millisecond timestamp")]
    Clock,
}

impl RecoveryError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::BootEvidence(_) => "boot_evidence_unreadable",
            Self::BootStampIo { .. } | Self::CheckpointIo { .. } | Self::JournalIo { .. } => {
                "persistence_failed"
            }
            Self::BootStampMalformed(_) => "boot_stamp_malformed",
            Self::CheckpointCorrupted { .. } => "checkpoint_corrupted",
            Self::CheckpointUnsupported { .. } => "checkpoint_unsupported",
            Self::CheckpointStale { .. } => "checkpoint_stale",
            Self::CheckpointTarget => "checkpoint_target_mismatch",
            Self::Engine(_) => "checkpoint_inconsistent",
            Self::HolderLink { .. } => "holder_unreachable",
            Self::HolderProtocol { .. } => "holder_protocol",
            Self::JournalCorrupt { .. } => "journal_corrupt",
            Self::Store(error) => error.kind(),
            Self::Clock => "clock_unrepresentable",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::BootEvidence(_) => {
                "survival stays unknown; nothing is relaunched until boot evidence is readable"
            }
            Self::BootStampIo { .. } | Self::CheckpointIo { .. } | Self::JournalIo { .. } => {
                "check the state directory's permissions and free space"
            }
            Self::BootStampMalformed(_) => {
                "inspect the boot stamp; survival stays unknown until it is settled"
            }
            Self::CheckpointCorrupted { .. } | Self::CheckpointUnsupported { .. } => {
                "inspect the quarantined checkpoint; the screen is rebuilt from retained output only"
            }
            Self::CheckpointStale { .. } => "checkpoint from the newest output offset only",
            Self::CheckpointTarget => "checkpoint the incarnation the store records",
            Self::Engine(_) => "report the checkpoint; it was written by an inconsistent engine",
            Self::HolderLink { .. } => {
                "survival of that holder group stays unknown; inspect its socket before relaunching"
            }
            Self::HolderProtocol { .. } => {
                "the holder speaks another build; adopt it with that build"
            }
            Self::JournalCorrupt { .. } => {
                "inspect the incident journal; it is never truncated or replaced"
            }
            Self::Store(error) => error.hint(),
            Self::Clock => "correct the runtime clock",
        }
    }
}
