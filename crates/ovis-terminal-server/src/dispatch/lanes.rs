//! Typed domain routes registered on the dispatcher by assembly.

pub mod agent_status;

pub mod conpty;
pub mod documents_retention;
pub mod machine_identity;
pub mod plugins;
pub mod prompt_bridge;
pub mod prompts;
pub mod recovery;
#[cfg(unix)]
pub mod restart;

pub mod session_binding;
#[cfg(unix)]
pub mod unix_service;
pub mod windows_service;
pub mod worktrees;
