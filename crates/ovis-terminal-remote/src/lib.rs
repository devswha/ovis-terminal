//! Authenticated remote machines over byte-transparent transports.
//!
//! Machines are keyed by the identity their runtime proves over a fresh challenge
//! ([`identity`]) and reached over transports that carry protocol bytes unchanged
//! ([`transport`]). Each machine has its own actor ([`actors`]); its events are mirrored
//! into the local sequence domain ([`mirror`]) and parent deaths reach it as orphan notices
//! kept until it acknowledges them ([`orphans`]).

pub mod actors;
pub mod identity;
pub mod mirror;
pub mod orphans;
pub mod transport;

#[cfg(test)]
pub mod qa_support;

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_core::CRATE_NAME,
    ovis_terminal_authz::CRATE_NAME,
    ovis_terminal_store::CRATE_NAME,
];
