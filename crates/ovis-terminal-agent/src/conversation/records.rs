//! Typed conversation records and turn records (plan todo 37).
//!
//! The owning process of a registered agent appends typed user, assistant and tool
//! records. Each append is checked against everything recorded before it: identities
//! stay unique, every tool result answers a call already made, and each record is
//! stamped with the status turn running when it arrived. Turn boundaries, outcome and
//! times are read from the agent status turn, so they are absent rather than guessed
//! when status did not record them. Nothing here reads terminal output.
//!
//! Every record is its own store entry, so the per-agent bounds (10,000 records,
//! 8 MiB) are this module's own counters, and an append's receipt carries identities,
//! counts and a digest instead of record bodies.

mod error;
mod read;
mod service;
mod store;
mod turns;
mod validate;

pub use error::{RecordsError, Violation};
pub use service::{Append, RecordsService};

#[cfg(test)]
mod qa;
