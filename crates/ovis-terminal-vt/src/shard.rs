//! Budgeted parse turns: one terminal's queued output fed to its [`Vt`](crate::Vt) in
//! bounded slices, so the owner yields to control work between turns.

mod budget;
mod error;
mod turn;

#[cfg(test)]
mod tests;

pub use budget::{ParseBudget, PendingCap};
pub use error::ShardError;
pub use turn::{Shard, Turn};
