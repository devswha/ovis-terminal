//! Latency and capacity benchmark harnesses for ovis-terminal.
//!
//! The pseudo-terminal harnesses live under `bench/harness/`; they run on Unix
//! hosts, where Linux and macOS are the first-class platforms.

#[cfg(unix)]
#[path = "../harness/harness.rs"]
pub mod harness;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_core::CRATE_NAME,
    ovis_terminal_vt::CRATE_NAME,
    ovis_terminal_probes::CRATE_NAME,
];
