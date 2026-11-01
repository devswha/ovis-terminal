//! Staged update vocabulary (plan todo 47): validated release values, the archive's
//! build manifest and the receipts that keep activation and rollback inspectable.
//!
//! The digest, target and version a caller names are parsed once here, so a version can
//! never name a path and a digest is always 64 lowercase hex characters.

mod records;
#[cfg(test)]
mod tests;
mod values;

pub use records::{
    ActivationPhase, ActivationReceipt, ActiveBuild, BuildManifest, PendingActivation,
    ProbeOutcome, ReadinessEvidence, RejectReason, RollbackReason, RollbackReceipt,
    UpdateRejection,
};
pub use values::{BuildTarget, HolderGeneration, ReleaseVersion, Sha256Digest, UpdateValueError};
