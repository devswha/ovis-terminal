//! PTY holder protocol and process ownership that survives control-server restarts.
//!
//! Scaffold crate: it compiles and records its place in the workspace layering;
//! runtime behavior arrives with the plan todos named on each module.

pub mod process;
pub mod protocol;
#[cfg(unix)]
pub mod unix;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[ovis_terminal_protocol::CRATE_NAME];
