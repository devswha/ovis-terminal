//! Worktree vocabulary shared by the service and dispatch lanes: record construction,
//! explicit removal scope, argv-safe branch names and the resources a refusal names.

use std::fmt;

use thiserror::Error;

use crate::errors::TargetId;
use crate::schema::ids::{CheckoutId, RepositoryId, WorkspaceId};
use crate::schema::methods::worktree::{
    RemoveScope, WorktreeChange, WorktreeListResult, WorktreeRecord,
};
use crate::schema::scalar::{EventSeq, Revision};

/// Every field of a [`WorktreeRecord`]; services outside this crate build records from it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "services in other crates build records from every field; a new field must fail their build"
)]
pub struct WorktreeFacts {
    /// Repository the checkout belongs to.
    pub repository_id: RepositoryId,
    /// Checkout identity, never derived from its path.
    pub checkout_id: CheckoutId,
    /// Canonical checkout path.
    pub path: String,
    /// Checked-out branch; `None` for a detached or bare checkout.
    pub branch: Option<BranchName>,
    /// Checkout this one was created from; `None` when it was not created here.
    pub source: Option<CheckoutId>,
    /// Uncommitted or untracked changes exist.
    pub dirty: bool,
    /// Git refuses to remove the checkout without a second force.
    pub locked: bool,
    /// Workspaces opened on the checkout.
    pub workspaces: Vec<WorkspaceId>,
    /// Sequence of the commit that last changed the checkout; zero if never changed here.
    pub revision: Revision,
}

impl From<WorktreeFacts> for WorktreeRecord {
    fn from(facts: WorktreeFacts) -> Self {
        Self {
            repository_id: facts.repository_id,
            checkout_id: facts.checkout_id,
            path: facts.path,
            branch: facts.branch.map(|branch| branch.0),
            source: facts.source,
            dirty: facts.dirty,
            locked: facts.locked,
            workspaces: facts.workspaces,
            revision: facts.revision,
        }
    }
}

impl WorktreeListResult {
    /// Lists checkouts in the order Git reports them.
    #[must_use]
    pub const fn new(worktrees: Vec<WorktreeRecord>) -> Self {
        Self { worktrees }
    }
}

impl WorktreeChange {
    /// A committed change, the workspaces it touched and its event sequence.
    #[must_use]
    pub const fn new(
        worktree: WorktreeRecord,
        affected_workspaces: Vec<WorkspaceId>,
        seq: EventSeq,
    ) -> Self {
        Self {
            worktree,
            affected_workspaces,
            seq,
        }
    }
}

impl RemoveScope {
    /// Group and force are accepted independently; neither implies the other.
    #[must_use]
    pub const fn new(group: bool, force: bool) -> Self {
        Self { group, force }
    }
}

/// A branch name that cannot be read as an option or carry control bytes into argv.
///
/// Git's own ref grammar is checked by the service with `git check-ref-format`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Accepts a short branch name such as `feature/login`.
    ///
    /// # Errors
    ///
    /// Returns `invalid_name` for an empty name, a leading dash, or whitespace and
    /// control characters.
    pub fn parse(raw: &str) -> Result<Self, BranchNameError> {
        if raw.is_empty() {
            return Err(BranchNameError::Empty);
        }
        if raw.starts_with('-') {
            return Err(BranchNameError::LeadingDash);
        }
        if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(BranchNameError::Control);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Borrows the short name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why a branch name was refused before any Git command ran.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BranchNameError {
    /// No name was given.
    #[error("the branch name is empty")]
    Empty,
    /// The name would be parsed as a Git option.
    #[error("the branch name starts with a dash")]
    LeadingDash,
    /// The name carries whitespace or control characters.
    #[error("the branch name contains whitespace or a control character")]
    Control,
}

impl BranchNameError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        "invalid_name"
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Empty => "name the new branch",
            Self::LeadingDash => "start the branch name with a letter or digit",
            Self::Control => "remove whitespace and control characters from the branch name",
        }
    }
}

/// One resource a refusal enumerates or a partial effect committed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AffectedResource {
    /// A checkout, by its stable identity.
    Checkout(CheckoutId),
    /// A workspace opened on a checkout.
    Workspace(WorkspaceId),
    /// A branch created for a checkout.
    Branch(BranchName),
}

impl AffectedResource {
    /// The identity carried in an error's `affected` list.
    #[must_use]
    pub fn target_id(&self) -> TargetId {
        TargetId::new(match self {
            Self::Checkout(checkout) => checkout.as_str().to_owned(),
            Self::Workspace(workspace) => workspace.as_str().to_owned(),
            Self::Branch(branch) => format!("branch:{branch}"),
        })
    }
}

#[cfg(test)]
mod tests;
