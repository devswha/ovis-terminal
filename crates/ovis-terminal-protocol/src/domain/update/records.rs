//! What a build declares, which build is active, and the receipts an activation or a
//! rollback leaves behind.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::values::{BuildTarget, HolderGeneration, ReleaseVersion, Sha256Digest};
use crate::errors::OpId;
use crate::schema::ids::UpdateId;
use crate::schema::methods::server::ServerTransitionResult;
use crate::schema::scalar::{OpenEnum, ProtocolGeneration, TimestampMs};

/// `build.json` at the root of a release archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BuildManifest {
    /// Release version of the build.
    pub version: ReleaseVersion,
    /// Target the build runs on.
    pub target: BuildTarget,
    /// Client protocol generations the build serves.
    pub protocol_generations: Vec<ProtocolGeneration>,
    /// Holder wire generations the build adopts and drains.
    pub holder_generations: Vec<HolderGeneration>,
}

impl BuildManifest {
    /// A manifest for `version` on `target` serving `generations`.
    #[must_use]
    pub fn new(
        version: ReleaseVersion,
        target: BuildTarget,
        (protocol, holder): (Vec<ProtocolGeneration>, Vec<HolderGeneration>),
    ) -> Self {
        Self {
            version,
            target,
            protocol_generations: protocol,
            holder_generations: holder,
        }
    }
}

/// A build an activation can select: its version, target and executable digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActiveBuild {
    /// Release version.
    pub version: ReleaseVersion,
    /// Target it runs on.
    pub target: BuildTarget,
    /// Digest of the executable, read from disk.
    pub binary_sha256: Sha256Digest,
}

impl ActiveBuild {
    /// The build `version` on `target` whose executable hashes to `binary_sha256`.
    #[must_use]
    pub const fn new(
        version: ReleaseVersion,
        target: BuildTarget,
        binary_sha256: Sha256Digest,
    ) -> Self {
        Self {
            version,
            target,
            binary_sha256,
        }
    }
}

/// Why a staged update was rejected; the active build is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RejectReason {
    /// The staged archive no longer hashes to the requested digest.
    DigestMismatch,
    /// An archive entry is absolute, escapes the build directory, is a link or repeats.
    UnsafeArchive,
    /// The archive lacks `build.json` or the executable.
    MissingEntry,
    /// `build.json` names another version or target than was staged.
    ManifestMismatch,
    /// The build does not serve this runtime's client protocol generation.
    ProtocolIncompatible,
    /// The build cannot adopt the running holders' wire generation.
    HolderIncompatible,
}

/// A rejection and the operation that found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UpdateRejection {
    /// What failed.
    pub reason: OpenEnum<RejectReason>,
    /// The operation that found it.
    pub op_id: OpId,
}

impl UpdateRejection {
    /// `reason`, found by `op_id`.
    #[must_use]
    pub const fn new(reason: RejectReason, op_id: OpId) -> Self {
        Self {
            reason: OpenEnum::Known(reason),
            op_id,
        }
    }
}

/// How the selected executable answered its readiness probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProbeOutcome {
    /// It reported the expected version and a protocol generation the runtime serves.
    Ready,
    /// It could not be started.
    SpawnFailed,
    /// It exited unsuccessfully.
    Exited,
    /// It exited successfully but reported another version or generation.
    Mismatched,
    /// It did not exit within the probe deadline and was killed.
    TimedOut,
}

/// Probe outcome and the bounded output that supports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadinessEvidence {
    /// How the probe ended.
    pub outcome: OpenEnum<ProbeOutcome>,
    /// Exit status, when the probe exited with one; absent for a signal or no exit.
    pub exit_code: Option<i32>,
    /// Leading standard output, lossily decoded and bounded.
    pub stdout: String,
}

impl ReadinessEvidence {
    /// `outcome` with `exit_code`, supported by `stdout`.
    #[must_use]
    pub fn new(outcome: ProbeOutcome, (exit_code, stdout): (Option<i32>, String)) -> Self {
        Self {
            outcome: OpenEnum::Known(outcome),
            exit_code,
            stdout,
        }
    }
}

/// Why the previous build was selected again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RollbackReason {
    /// The selected executable failed its readiness probe.
    ReadinessFailed,
    /// The control restart onto the selected build failed.
    RestartFailed,
    /// An activation was interrupted and finished by recovery.
    Interrupted,
    /// A caller asked for the rollback.
    Requested,
}

/// The rollback of one activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RollbackReceipt {
    /// Operation that rolled back.
    pub op_id: OpId,
    /// Why.
    pub reason: OpenEnum<RollbackReason>,
    /// Build that was deselected.
    pub from: ActiveBuild,
    /// Build selected again.
    pub restored: ActiveBuild,
    /// Probe evidence, when a probe ran.
    pub readiness: Option<ReadinessEvidence>,
    /// Control restart onto the restored build, when one ran.
    pub restart: Option<ServerTransitionResult>,
    /// When it was recorded.
    pub recorded_ms: TimestampMs,
}

impl RollbackReceipt {
    /// `op_id` selected `restored` again in place of `from` at `recorded_ms`.
    #[must_use]
    pub fn new(
        op_id: OpId,
        reason: RollbackReason,
        (from, restored, recorded_ms): (ActiveBuild, ActiveBuild, TimestampMs),
    ) -> Self {
        Self {
            op_id,
            reason: OpenEnum::Known(reason),
            from,
            restored,
            readiness: None,
            restart: None,
            recorded_ms,
        }
    }
}

/// The successful activation of one update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActivationReceipt {
    /// Operation that activated it.
    pub op_id: OpId,
    /// Build it replaced, which a rollback restores.
    pub previous: ActiveBuild,
    /// Readiness evidence of the new build.
    pub readiness: ReadinessEvidence,
    /// Control restart onto the new build; holders keep their programs and drain.
    pub restart: ServerTransitionResult,
    /// When it was recorded.
    pub recorded_ms: TimestampMs,
}

impl ActivationReceipt {
    /// `op_id` replaced `previous`, proven by `evidence` and recorded at `recorded_ms`.
    #[must_use]
    pub fn new(
        op_id: OpId,
        previous: ActiveBuild,
        (readiness, restart, recorded_ms): (ReadinessEvidence, ServerTransitionResult, TimestampMs),
    ) -> Self {
        Self {
            op_id,
            previous,
            readiness,
            restart,
            recorded_ms,
        }
    }
}

/// Durable steps of an activation; recovery finishes or reverts from the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivationPhase {
    /// The intent is durable; the selection may not have changed yet.
    Intent,
    /// The new build is selected; readiness and restart are unproven.
    Selected,
    /// Readiness and restart succeeded; only the record remains to be written.
    Ready,
}

/// An activation recovery has not finished yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PendingActivation {
    /// Update being activated.
    pub update_id: UpdateId,
    /// Activating operation.
    pub op_id: OpId,
    /// Last durable step.
    pub phase: OpenEnum<ActivationPhase>,
}

impl PendingActivation {
    /// `op_id` activating `update_id`, last durable at `phase`.
    #[must_use]
    pub const fn new(update_id: UpdateId, op_id: OpId, phase: ActivationPhase) -> Self {
        Self {
            update_id,
            op_id,
            phase: OpenEnum::Known(phase),
        }
    }
}
