//! The epoch of one dispatched grid; the holder refuses any epoch older than the one it applied.

use ovis_terminal_protocol::rpc::DecimalU64;
use serde::{Deserialize, Serialize};

use super::GeometryError;

/// Position of a grid change dispatched to the holder, carried as a decimal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GeometryEpoch(DecimalU64);

impl GeometryEpoch {
    /// Wraps an epoch value; zero is the grid the program was spawned with.
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// The epoch value.
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) fn next(self) -> Result<Self, GeometryError> {
        self.get()
            .checked_add(1)
            .map(Self::new)
            .ok_or(GeometryError::Exhausted)
    }
}
