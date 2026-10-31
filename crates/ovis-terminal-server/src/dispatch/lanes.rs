//! Typed domain routes registered on the dispatcher by assembly.

pub mod agent_status;

pub mod conpty;
pub mod conversation_records;
pub mod documents_retention;
pub mod machine_identity;
pub mod plugins;
pub mod prompt_bridge;
pub mod prompts;
#[cfg(unix)]
pub mod restart;

pub mod session_binding;
pub mod template_launch;
#[cfg(unix)]
pub mod unix_service;
pub mod windows_service;
pub mod worktrees;
