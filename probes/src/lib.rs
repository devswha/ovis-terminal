//! Native platform probes for PTY, process and VT behavior.
//!
//! Scaffold crate: harness code arrives with plan todo 12.

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_holder::CRATE_NAME,
    ovis_terminal_vt::CRATE_NAME,
];
