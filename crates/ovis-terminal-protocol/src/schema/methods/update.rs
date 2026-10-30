//! `update.*`: staged, verified updates with inspectable activation and rollback.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::common::NoParams;
use crate::schema::ids::{BuildVersion, Digest, UpdateId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{OpenEnum, ProtocolGeneration};

/// Progress of a staged update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UpdateState {
    /// Downloaded into the install root.
    Staged,
    /// Integrity and compatibility checks passed.
    Verified,
    /// Selected as the active build.
    Activated,
    /// Activation failed and the previous build was restored.
    RolledBack,
    /// A check failed; the active build is unchanged.
    Rejected,
}

/// One staged update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct UpdateRecord {
    /// Update identity.
    pub update_id: UpdateId,
    /// Target build version.
    pub version: BuildVersion,
    /// Content digest.
    pub digest: Digest,
    /// Protocol generation of the target build.
    pub generation: ProtocolGeneration,
    /// Progress.
    pub state: OpenEnum<UpdateState>,
    /// Build that stays active until activation succeeds.
    pub active_version: BuildVersion,
}

/// Parameters of `update.stage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UpdateStageParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Build version to stage.
    pub version: BuildVersion,
    /// Expected content digest.
    pub digest: Digest,
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
#[non_exhaustive]
pub struct UpdateQueryResult {
    /// Active build.
    pub active_version: BuildVersion,
    /// Staged updates.
    pub updates: Vec<UpdateRecord>,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"update.stage", Mutation, Untargeted, UpdateStageParams => UpdateRecord,
    "Stage a build in the install root without touching the active one.", [IntegrityFailed, TransportFailed]},
    method! {"update.verify", Mutation, Lookup, UpdateStepParams => UpdateRecord,
    "Check digest and protocol and holder compatibility.", [IntegrityFailed, Incompatible]},
    method! {"update.activate", Mutation, Lookup, UpdateStepParams => UpdateRecord,
    "Atomically select a verified build; holders drain by build.", [NoStagedUpdate, Incompatible, RollbackFailed]},
    method! {"update.rollback", Mutation, Lookup, UpdateStepParams => UpdateRecord,
    "Restore the previous build.", [RollbackFailed]},
    method! {"update.query", Query, Untargeted, NoParams => UpdateQueryResult,
    "Read the active build and staged updates.", []},
];
