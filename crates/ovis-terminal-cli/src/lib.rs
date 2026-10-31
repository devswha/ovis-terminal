//! Agent-facing ovis-terminal command-line interface.
//!
//! An invocation is parsed and planned completely before anything outside the process is
//! touched, writes one schema-defined result to stdout or one structured error to stderr,
//! and exits with a published code. No command starts, installs or restarts a server.

mod invocation;

pub use invocation::{CREDENTIAL_ENV, Context, Credential, ExitStatus, JSON_ENV, Report, run};

/// Name of the installed binary.
pub const BINARY_NAME: &str = "ovis-terminal";

/// Release version of the binary, taken from the workspace package version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_server::CRATE_NAME,
];
