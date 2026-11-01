//! Typed construction of published records owned by domain services.

pub mod agent_status;
pub mod agent_tree;

pub mod conpty;
pub mod conversation_records;
pub mod credential_rotation;
pub mod documents_retention;
pub mod health;
pub mod machine_actor;
pub mod machine_identity;
pub mod plugins;
pub mod process_info;
pub mod prompt_bridge;
pub mod prompts;
pub mod recovery;
pub mod restart;

pub mod session_binding;
pub mod template_launch;
pub mod unix_service;
pub mod update;
pub mod windows_service;
pub mod worktrees;
