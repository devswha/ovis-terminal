//! Typed domain routes registered on the dispatcher by assembly.

pub mod agent_status;

pub mod conpty;
pub mod documents_retention;
pub mod machine_identity;
pub mod plugins;

#[cfg(unix)]
pub mod unix_service;
pub mod worktrees;
