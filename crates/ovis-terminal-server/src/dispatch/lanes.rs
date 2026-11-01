//! Typed domain routes registered on the dispatcher by assembly.

pub mod agent_status;
pub mod agent_tree;

pub mod conpty;
pub mod credential_rotation;
pub mod documents_retention;
pub mod machine_identity;
pub mod plugins;
pub mod process_info;
pub mod prompt_bridge;
pub mod prompts;
pub mod recovery;
#[cfg(unix)]
pub mod restart;

pub mod session_binding;
pub mod template_launch;
#[cfg(unix)]
pub mod unix_service;
pub mod update;
pub mod windows_service;
pub mod worktrees;
