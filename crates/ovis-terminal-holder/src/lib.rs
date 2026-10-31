//! PTY holder protocol and process ownership that survives control-server restarts.
//!
//! [`protocol`] is the frozen holder wire contract, [`process`] gathers process
//! incarnation evidence, [`unix`] owns PTYs and their programs on Unix, and `windows`
//! owns ConPTY pseudoconsoles on Windows.

pub mod process;
pub mod protocol;
pub mod service;
#[cfg(unix)]
pub mod unix;
#[cfg(any(windows, test, feature = "ffi-model"))]
pub mod windows;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[ovis_terminal_protocol::CRATE_NAME];
