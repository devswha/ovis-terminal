//! The admission position of an entry in one terminal's input sequence.

use ovis_terminal_protocol::rpc::DecimalU64;
use serde::{Deserialize, Serialize};

/// Position of an admitted entry in one terminal's input sequence, carried as a decimal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InputSeq(DecimalU64);

impl InputSeq {
    /// Wraps a sequence value.
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// The sequence value.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}
