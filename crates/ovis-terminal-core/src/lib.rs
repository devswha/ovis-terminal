//! Control core: identities, lifecycle, events, leases, input ordering and layout.
//!
//! Scaffold crate: it compiles and records its place in the workspace layering;
//! runtime behavior arrives with the plan todos named on each module.

pub mod docs;
pub mod events;
pub mod geometry;
pub mod input;
pub mod layout;
pub mod leases;
pub mod ops;
pub mod registry;
pub mod retention;
pub mod screen;
pub mod subscriptions;
pub mod telemetry;
pub mod terminals;
pub mod waits;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_store::CRATE_NAME,
    ovis_terminal_vt::CRATE_NAME,
    ovis_terminal_holder::CRATE_NAME,
    ovis_terminal_authz::CRATE_NAME,
];
