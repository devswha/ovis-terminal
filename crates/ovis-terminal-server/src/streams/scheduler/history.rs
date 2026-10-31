//! Disk-backed output history: one terminal's raw output in append-only segment files.
//!
//! Each segment is a file named by the 20-digit decimal offset of its first byte and
//! holds the following bytes contiguously; a new segment starts once the current one
//! reaches the segment size. The index is the in-memory list of segments, rebuilt from
//! the directory on open and bounded by retention: once the retained bytes pass the
//! retention bound, the oldest segments are deleted and the floor (the first offset still
//! readable) advances. A read below the floor is a typed `history_gap`, never empty
//! success. The store runs on the terminal's dedicated writer thread, never on an async
//! worker.

mod error;
mod store;

#[cfg(test)]
mod tests;

pub use error::HistoryError;
pub use store::{HistoryConfig, HistoryRead, HistoryStats, HistoryStore};
