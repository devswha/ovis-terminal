//! The worktree boundary's typed failures; refusals name every affected resource.

use std::path::PathBuf;

use ovis_terminal_protocol::domain::worktrees::{AffectedResource, BranchNameError};
use ovis_terminal_protocol::schema::ids::{CheckoutId, RepositoryId};

use super::workspaces::WorkspaceFailure;

/// A worktree request that was refused, failed, or stopped part way.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorktreeError {
    /// No repository is registered under this identity.
    #[error("the repository identity is not registered")]
    UnknownRepository(RepositoryId),
    /// No live checkout has this identity in the named repository.
    #[error("the checkout identity names no live checkout")]
    UnknownCheckout(CheckoutId),
    /// The path lies outside the authorized root or is not representable on the wire.
    #[error("the path is outside the authorized root")]
    PathForbidden {
        /// The refused path.
        path: PathBuf,
    },
    /// The repository's primary checkout is never removed.
    #[error("the primary checkout of a repository cannot be removed")]
    PrimaryCheckout(CheckoutId),
    /// A configured path could not be resolved.
    #[error("a configured path could not be resolved")]
    Path {
        /// The unresolved path.
        path: PathBuf,
        /// Resolution failure.
        #[source]
        source: std::io::Error,
    },
    /// The branch name failed the argv-safety check.
    #[error("the branch name is not acceptable")]
    Branch(#[from] BranchNameError),
    /// Git's ref grammar refuses the branch name.
    #[error("git refuses the branch name")]
    RefFormat,
    /// The branch or the checkout directory already exists.
    #[error("the branch or checkout directory already exists")]
    Exists {
        /// What already exists.
        affected: Vec<AffectedResource>,
    },
    /// Linked workspaces would close, which needs group scope.
    #[error("removal would close linked workspaces without group scope")]
    ScopeRequired {
        /// The checkout and every linked workspace.
        affected: Vec<AffectedResource>,
    },
    /// The checkout has uncommitted or untracked changes, which needs force scope.
    #[error("the checkout has uncommitted changes and force scope was not given")]
    Dirty {
        /// The checkout and every linked workspace.
        affected: Vec<AffectedResource>,
    },
    /// The checkout is locked, which needs force scope.
    #[error("the checkout is locked and force scope was not given")]
    Locked {
        /// The checkout and every linked workspace.
        affected: Vec<AffectedResource>,
    },
    /// Git could not be started or awaited.
    #[error("git could not run")]
    Spawn {
        /// The Git step.
        operation: &'static str,
        /// Process failure.
        #[source]
        source: std::io::Error,
    },
    /// Git exited unsuccessfully.
    #[error("git exited unsuccessfully")]
    Vcs {
        /// The Git step.
        operation: &'static str,
        /// Exit code, absent when killed by a signal.
        code: Option<i32>,
    },
    /// The checkout's identity marker could not be read or written.
    #[error("the checkout identity marker is unreadable or unwritable")]
    Marker {
        /// The marker path.
        path: PathBuf,
        /// I/O or decoding failure.
        #[source]
        source: std::io::Error,
    },
    /// Git output did not match the porcelain format.
    #[error("git output is not in the porcelain format")]
    Porcelain,
    /// Git exceeded its bound and was killed.
    #[error("git exceeded its time bound and was killed")]
    TimedOut {
        /// The Git step.
        operation: &'static str,
        /// The bound in milliseconds.
        bound_ms: u64,
    },
    /// The operation was cancelled; `committed` lists what had already happened.
    #[error("the operation was cancelled")]
    Interrupted {
        /// Effects committed before the cancellation.
        committed: Vec<AffectedResource>,
    },
    /// The workspace service failed before anything changed.
    #[error("the workspace service failed")]
    Workspaces(#[source] WorkspaceFailure),
    /// A later step failed after earlier effects committed.
    #[error("the request failed after part of it was applied")]
    Partial {
        /// Effects that committed.
        committed: Vec<AffectedResource>,
        /// The failing step.
        #[source]
        source: Box<Self>,
    },
}

impl WorktreeError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnknownRepository(_) | Self::UnknownCheckout(_) => "not_found",
            Self::PathForbidden { .. } | Self::PrimaryCheckout(_) => "path_forbidden",
            Self::Path { .. } => "path_not_found",
            Self::Branch(_) | Self::RefFormat => "invalid_name",
            Self::Exists { .. } => "duplicate_name",
            Self::ScopeRequired { .. } => "scope_required",
            Self::Dirty { .. } => "checkout_dirty",
            Self::Locked { .. } => "checkout_locked",
            Self::Spawn { .. }
            | Self::Vcs { .. }
            | Self::Porcelain
            | Self::Marker { .. }
            | Self::TimedOut { .. } => "vcs_failed",
            Self::Interrupted { .. } => "cancelled",
            Self::Workspaces(_) => "unavailable",
            Self::Partial { .. } => "outcome_unknown",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::UnknownRepository(_) => "name a repository returned by the runtime configuration",
            Self::UnknownCheckout(_) => "list the repository's worktrees and name a live checkout",
            Self::PathForbidden { .. } => "keep checkouts inside the authorized root",
            Self::PrimaryCheckout(_) => "name a linked checkout; the primary checkout stays",
            Self::Path { .. } => "configure an existing directory",
            Self::Branch(error) => error.hint(),
            Self::RefFormat => "name a branch that git check-ref-format --branch accepts",
            Self::Exists { .. } => "choose a branch name that is not in use",
            Self::ScopeRequired { .. } => {
                "review the affected workspaces and retry with group scope"
            }
            Self::Dirty { .. } => "commit or discard the changes, or retry with force scope",
            Self::Locked { .. } => "unlock the checkout, or retry with force scope",
            Self::Spawn { .. } => "install git and make it executable by the runtime",
            Self::Vcs { .. } | Self::Porcelain => "inspect the repository with git and retry",
            Self::Marker { .. } => {
                "repair or remove the ovis marker in the checkout's git directory"
            }
            Self::TimedOut { .. } => "inspect the repository for a held lock and retry",
            Self::Interrupted { .. } => "inspect the affected resources; they were committed",
            Self::Workspaces(_) => "retry once the workspace service is available",
            Self::Partial { .. } => "inspect the affected resources and retry the remainder",
        }
    }

    /// Whether the same request may succeed later.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::TimedOut { .. } | Self::Workspaces(_))
    }

    /// Resources the refusal protects or the partial effect committed.
    #[must_use]
    pub fn affected(&self) -> &[AffectedResource] {
        match self {
            Self::Exists { affected }
            | Self::ScopeRequired { affected }
            | Self::Dirty { affected }
            | Self::Locked { affected }
            | Self::Interrupted {
                committed: affected,
            }
            | Self::Partial {
                committed: affected,
                ..
            } => affected,
            _ => &[],
        }
    }
}
