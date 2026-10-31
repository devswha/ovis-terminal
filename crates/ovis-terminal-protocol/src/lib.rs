//! Wire framing, correlated JSON-RPC envelopes, boundary errors and the OpenRPC schema of ovis-terminal.
//!
//! Scaffold crate: it compiles and records its place in the workspace layering;
//! runtime behavior arrives with the plan todos named on each module.

pub mod domain;
pub mod errors;
pub mod rpc;
pub mod schema;
pub mod wire;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[];
