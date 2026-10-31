//! Explicit worktree identities and destructive scope over argv-invoked Git (plan todo 45).
//!
//! Repositories are registered by identity inside one authorized root; checkouts get
//! identities that never derive from their path. Removal enumerates every affected
//! resource and needs `group` and `force` independently; refusals touch nothing.

mod cancel;
mod create;
mod error;
mod event;
mod git;
mod marker;
mod porcelain;
mod remove;
mod scope;
mod service;
mod state;
mod workspaces;

#[cfg(all(test, unix))]
mod tests;

pub use cancel::Cancel;
pub use error::WorktreeError;
pub use event::{Change, WorktreeEvent};
pub use git::Git;
pub use service::{Call, Config, Repository, Worktrees};
pub use workspaces::{WorkspaceFailure, Workspaces};
