//! Advertised retention with durable, visible floors (plan todo 50).
//!
//! A sweep evicts the records a [`RetentionLimit`] no longer covers at a supplied clock
//! reading, and commits the advanced floor, a tombstone per evicted record and one
//! `retention.floor_advanced` event in a single transaction. Floors never move backwards,
//! and reads below a floor are reported as expired or as a gap, never as absent data.
//!
//! [`RetentionLimit`]: ovis_terminal_protocol::domain::documents_retention::RetentionLimit

mod error;
mod event;
mod floors;
mod sweep;

pub use error::RetentionError;
pub use event::FloorAdvanced;
pub use floors::floors;
pub use sweep::{Candidate, Eviction, Sweep, SweepOutcome, plan};
