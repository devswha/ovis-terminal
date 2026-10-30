//! `worktree.*`: explicit repository and checkout identities with destructive scope.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::ids::{CheckoutId, RepositoryId, WorkspaceId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::{EventSeq, Revision};

/// One checkout of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WorktreeRecord {
    /// Repository.
    pub repository_id: RepositoryId,
    /// Checkout.
    pub checkout_id: CheckoutId,
    /// Checkout path.
    pub path: String,
    /// Checked-out branch, when any.
    pub branch: Option<String>,
    /// Checkout this one was created from.
    pub source: Option<CheckoutId>,
    /// Whether the checkout has uncommitted changes.
    pub dirty: bool,
    /// Whether the checkout is locked.
    pub locked: bool,
    /// Workspaces opened on this checkout.
    pub workspaces: Vec<WorkspaceId>,
    /// Record revision.
    pub revision: Revision,
}

/// Parameters of `worktree.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WorktreeListParams {
    /// Repository.
    pub repository_id: RepositoryId,
}

/// Result of `worktree.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WorktreeListResult {
    /// Checkouts.
    pub worktrees: Vec<WorktreeRecord>,
}

/// Parameters of `worktree.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WorktreeCreateParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Repository.
    pub repository_id: RepositoryId,
    /// Checkout to branch from.
    pub source: CheckoutId,
    /// New branch name.
    pub branch: String,
}

/// Parameters of `worktree.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WorktreeOpenParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Checkout.
    pub checkout_id: CheckoutId,
}

/// Destructive scope `worktree.remove` explicitly accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RemoveScope {
    /// Also close linked workspaces.
    pub group: bool,
    /// Remove even when dirty or locked.
    pub force: bool,
}

/// Parameters of `worktree.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WorktreeRemoveParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Checkout.
    pub checkout_id: CheckoutId,
    /// Accepted destructive scope.
    pub scope: RemoveScope,
}

/// Result of worktree mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WorktreeChange {
    /// Checkout after the change.
    pub worktree: WorktreeRecord,
    /// Workspaces the change affected.
    pub affected_workspaces: Vec<WorkspaceId>,
    /// Sequence of the commit event.
    pub seq: EventSeq,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"worktree.list", Query, Lookup, WorktreeListParams => WorktreeListResult,
    "List checkouts with branch, source, dirty and locked state.", []},
    method! {"worktree.create", Mutation, Lookup, WorktreeCreateParams => WorktreeChange,
    "Create a checkout from an explicit source.", [VcsFailed, PathForbidden]},
    method! {"worktree.open", Mutation, Lookup, WorktreeOpenParams => WorktreeChange,
    "Open a workspace on an explicit checkout.", []},
    method! {"worktree.remove", Mutation, Lookup, WorktreeRemoveParams => WorktreeChange,
    "Remove a checkout after enumerating affected resources.", [ScopeRequired, CheckoutDirty, CheckoutLocked, VcsFailed]},
];
