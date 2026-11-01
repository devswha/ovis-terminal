//! Native platform probes for PTY, process and VT behavior.
//!
//! The probes live under `probes/platform/`; they drive real pseudo-terminals
//! through the pinned portable-pty backend and report measured behavior or a
//! typed [`platform::ProbeError`].

#[path = "../platform/platform.rs"]
pub mod platform;

#[path = "../quality/quality.rs"]
pub mod quality;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_holder::CRATE_NAME,
    ovis_terminal_vt::CRATE_NAME,
];
