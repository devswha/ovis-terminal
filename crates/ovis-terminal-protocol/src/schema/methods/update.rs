//! `update.*`: staged, verified updates with inspectable activation and rollback.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::update::{
    ActivationReceipt, ActiveBuild, BuildManifest, BuildTarget, PendingActivation, ReleaseVersion,
    RollbackReceipt, Sha256Digest, UpdateRejection,
};
use crate::errors::OpId;
use crate::schema::common::NoParams;
use crate::schema::ids::UpdateId;
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{Incarnation, OpenEnum};

/// Progress of a staged update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UpdateState {
    /// Copied into the install root with the requested digest.
    Staged,
    /// Integrity and compatibility checks passed.
    Verified,
    /// Selected as the active build.
    Activated,
    /// Deselected again; the rollback receipt says why.
    RolledBack,
    /// A check failed; the active build is unchanged.
    Rejected,
}

/// A build that passed verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct VerifiedBuild {
    /// The archive's `build.json`.
    pub manifest: BuildManifest,
    /// Digest of the extracted executable.
    pub binary_sha256: Sha256Digest,
}

impl VerifiedBuild {
    /// `manifest` with an executable hashing to `binary_sha256`.
    #[must_use]
    pub const fn new(manifest: BuildManifest, binary_sha256: Sha256Digest) -> Self {
        Self {
            manifest,
            binary_sha256,
        }
    }
}

/// One staged update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UpdateRecord {
    /// Update identity.
    pub update_id: UpdateId,
    /// Target build version.
    pub version: ReleaseVersion,
    /// Target the build runs on.
    pub target: BuildTarget,
    /// Digest of the staged release archive.
    pub sha256: Sha256Digest,
    /// Progress.
    pub state: OpenEnum<UpdateState>,
    /// Build active when this record was last written.
    pub active_version: ReleaseVersion,
    /// Operation that staged it.
    pub staged_by: OpId,
    /// Verified build facts; absent until verification passes.
    pub build: Option<VerifiedBuild>,
    /// Why it was rejected, when it was.
    pub rejection: Option<UpdateRejection>,
    /// Successful activation, when one happened.
    pub activation: Option<ActivationReceipt>,
    /// Latest rollback, when one happened.
    pub rollback: Option<RollbackReceipt>,
}

impl UpdateRecord {
    /// A freshly staged `release` while `active_version` stays active.
    #[must_use]
    pub fn staged(
        update_id: UpdateId,
        release: (ReleaseVersion, BuildTarget, Sha256Digest),
        (active_version, staged_by): (ReleaseVersion, OpId),
    ) -> Self {
        let (version, target, sha256) = release;
        Self {
            update_id,
            version,
            target,
            sha256,
            state: OpenEnum::Known(UpdateState::Staged),
            active_version,
            staged_by,
            build: None,
            rejection: None,
            activation: None,
            rollback: None,
        }
    }
}

/// Parameters of `update.stage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UpdateStageParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Build version to stage.
    pub version: ReleaseVersion,
    /// Target of the release; absent means this runtime's target, and any other is refused.
    pub target: Option<BuildTarget>,
    /// Expected digest of the release archive.
    pub sha256: Sha256Digest,
}

/// Parameters of update transitions on a staged update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UpdateStepParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Update.
    pub update_id: UpdateId,
}

/// Result of `update.query`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UpdateQueryResult {
    /// Active build version.
    pub active_version: ReleaseVersion,
    /// Active build, with the digest of its executable as read now.
    pub active: ActiveBuild,
    /// Build a rollback would select again; absent before the first activation.
    pub previous: Option<ActiveBuild>,
    /// An activation whose recovery has not finished.
    pub pending: Option<PendingActivation>,
    /// Program incarnations the holders keep across activations.
    pub live_incarnations: Vec<Incarnation>,
    /// Staged updates, oldest first.
    pub updates: Vec<UpdateRecord>,
}

impl UpdateQueryResult {
    /// `active` and its rollback target with the staged `updates`.
    #[must_use]
    pub fn new(
        (active, previous): (ActiveBuild, Option<ActiveBuild>),
        pending: Option<PendingActivation>,
        (live_incarnations, updates): (Vec<Incarnation>, Vec<UpdateRecord>),
    ) -> Self {
        Self {
            active_version: active.version.clone(),
            active,
            previous,
            pending,
            live_incarnations,
            updates,
        }
    }
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"update.stage", Mutation, Untargeted, UpdateStageParams => UpdateRecord,
    "Stage a build in the install root without touching the active one.", [IntegrityFailed, Incompatible, TransportFailed, TooLarge]},
    method! {"update.verify", Mutation, Lookup, UpdateStepParams => UpdateRecord,
    "Check digest, archive safety and protocol and holder compatibility.", [NoStagedUpdate, InvalidUpdateState, IntegrityFailed, Incompatible]},
    method! {"update.activate", Mutation, Lookup, UpdateStepParams => UpdateRecord,
    "Atomically select a verified build; holders drain by build.", [NoStagedUpdate, InvalidUpdateState, IntegrityFailed, Incompatible, ActivationFailed, RollbackFailed]},
    method! {"update.rollback", Mutation, Lookup, UpdateStepParams => UpdateRecord,
    "Restore the previous build.", [NoStagedUpdate, InvalidUpdateState, RollbackFailed]},
    method! {"update.query", Query, Untargeted, NoParams => UpdateQueryResult,
    "Read the active build and staged updates.", []},
];
