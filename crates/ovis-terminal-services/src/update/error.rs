//! The update boundary's typed failures and whether each one is a proven outcome.

use std::path::PathBuf;

use ovis_terminal_protocol::domain::update::{
    BuildTarget, RejectReason, RollbackReason, Sha256Digest, UpdateValueError,
};
use ovis_terminal_protocol::schema::ids::UpdateId;

use super::control::RestartFailure;

/// An update step that was refused, failed, or stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UpdateError {
    /// No staged update has this identity.
    #[error("no staged update has this identity")]
    NoStagedUpdate(UpdateId),
    /// The update is not in the state this step starts from.
    #[error("the update is not in the state this step starts from")]
    InvalidState {
        /// Step that was asked for.
        step: &'static str,
    },
    /// An activation or rollback is still being recovered.
    #[error("an earlier activation is still being recovered")]
    ActivationPending,
    /// The release names another target than this runtime's.
    #[error("the release target is not this runtime's target")]
    WrongTarget {
        /// Target that was asked for.
        requested: BuildTarget,
        /// Target this runtime runs on.
        host: BuildTarget,
    },
    /// The release source has no archive for the version and target.
    #[error("the release source has no archive for this version and target")]
    ReleaseMissing {
        /// Where the archive was looked for.
        path: PathBuf,
        /// Open failure.
        #[source]
        source: std::io::Error,
    },
    /// The archive exceeds its size bound.
    #[error("the release archive exceeds its size bound")]
    TooLarge {
        /// Bound in bytes.
        limit: u64,
    },
    /// The copied archive does not hash to the requested digest.
    #[error("the release archive does not hash to the requested digest")]
    DigestMismatch {
        /// Requested digest.
        expected: Sha256Digest,
        /// Digest of the bytes read.
        actual: Sha256Digest,
    },
    /// Verification rejected the staged build; the active build is unchanged.
    #[error("verification rejected the staged build")]
    Rejected(RejectReason),
    /// The selected build failed and the previous build was selected again.
    #[error("the selected build failed and the previous build was restored")]
    ActivationFailed(RollbackReason),
    /// The previous build could not be selected again.
    #[error("the previous build could not be selected again")]
    RollbackFailed {
        /// Why.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The control restart failed.
    #[error("the control restart failed")]
    Restart(#[from] RestartFailure),
    /// The operation was cancelled; partial files were removed.
    #[error("the operation was cancelled before it changed anything")]
    Cancelled,
    /// An install-root file could not be read or written.
    #[error("an install-root file could not be read or written")]
    Io {
        /// What was being done.
        operation: &'static str,
        /// I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// An install-root record is not valid JSON of its type.
    #[error("an install-root record is corrupt")]
    Record(#[from] serde_json::Error),
    /// A thread panicked while holding the update state.
    #[error("the update state was poisoned by a panic")]
    Poisoned,
    /// A computed value failed its own grammar, which only a defect can cause.
    #[error("a computed update value is malformed")]
    Value(#[from] UpdateValueError),
    /// The readiness probe needs a Tokio runtime and none is current.
    #[error("no Tokio runtime is available for the readiness probe")]
    NoRuntime(#[from] tokio::runtime::TryCurrentError),
}

impl UpdateError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NoStagedUpdate(_) => "no_staged_update",
            Self::InvalidState { .. } | Self::ActivationPending => "invalid_update_state",
            Self::WrongTarget { .. } => "incompatible",
            Self::ReleaseMissing { .. } => "transport_failed",
            Self::TooLarge { .. } => "too_large",
            Self::DigestMismatch { .. } => "integrity_failed",
            Self::Rejected(reason) => rejection_kind(*reason),
            Self::ActivationFailed(_) => "activation_failed",
            Self::RollbackFailed { .. } => "rollback_failed",
            Self::Restart(failure) => failure.kind(),
            Self::Cancelled => "cancelled",
            Self::Io { .. } | Self::Record(_) => "persistence_failed",
            Self::NoRuntime(_) | Self::Value(_) | Self::Poisoned => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NoStagedUpdate(_) => "list staged updates with update.query",
            Self::InvalidState { .. } => "read the update's state with update.query first",
            Self::ActivationPending => "restart the runtime so recovery can finish it",
            Self::WrongTarget { .. } => "stage the release built for this runtime's target",
            Self::ReleaseMissing { .. } => "publish the archive in the release source first",
            Self::TooLarge { .. } => "stage a release within the archive size bound",
            Self::DigestMismatch { .. } => "check the release checksum and stage it again",
            Self::Rejected(reason) => rejection_hint(*reason),
            Self::ActivationFailed(_) => "update.query shows the rollback receipt and evidence",
            Self::RollbackFailed { .. } => "the selection is unchanged; inspect the install root",
            Self::Restart(failure) => failure.hint(),
            Self::Cancelled => "stage the release again when needed",
            Self::Io { .. } | Self::Record(_) => "check the install root's disk and permissions",
            Self::NoRuntime(_) => "run update steps on the runtime's blocking pool",
            Self::Value(_) | Self::Poisoned => "report this runtime build as defective",
        }
    }

    /// Whether the failure leaves a known outcome; uncertain ones may have left effects.
    #[must_use]
    pub const fn proven(&self) -> bool {
        !matches!(
            self,
            Self::Io { .. }
                | Self::Record(_)
                | Self::NoRuntime(_)
                | Self::Restart(_)
                | Self::Value(_)
                | Self::Poisoned
        )
    }
}

const fn rejection_kind(reason: RejectReason) -> &'static str {
    match reason {
        RejectReason::ProtocolIncompatible | RejectReason::HolderIncompatible => "incompatible",
        RejectReason::DigestMismatch
        | RejectReason::UnsafeArchive
        | RejectReason::MissingEntry
        | RejectReason::ManifestMismatch
        | _ => "integrity_failed",
    }
}

const fn rejection_hint(reason: RejectReason) -> &'static str {
    match reason {
        RejectReason::ProtocolIncompatible => "stage a build that serves this protocol generation",
        RejectReason::HolderIncompatible => "stage a build that adopts the running holders",
        RejectReason::UnsafeArchive => "the archive may only hold plain files inside its root",
        RejectReason::MissingEntry => "the archive needs build.json and the executable",
        RejectReason::DigestMismatch | RejectReason::ManifestMismatch | _ => {
            "stage the release again from a trusted source"
        }
    }
}
