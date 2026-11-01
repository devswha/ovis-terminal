//! Latency and capacity benchmark harnesses for ovis-terminal.
//!
//! The pseudo-terminal harnesses live under `bench/harness/`; they run on Unix
//! hosts, where Linux and macOS are the first-class platforms.

#[cfg(unix)]
#[path = "../harness/harness.rs"]
pub mod harness;

#[cfg(unix)]
#[path = "../tuning/tuning.rs"]
pub mod tuning;

#[cfg(unix)]
#[path = "../terminal50/terminal50.rs"]
pub mod terminal50;

#[cfg(unix)]
#[path = "../soak/soak.rs"]
pub mod soak;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_core::CRATE_NAME,
    ovis_terminal_holder::CRATE_NAME,
    ovis_terminal_vt::CRATE_NAME,
    ovis_terminal_probes::CRATE_NAME,
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_server::CRATE_NAME,
];
