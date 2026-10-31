//! Whether an owned PID plus start token still names a running process.

use super::table::ProcessTable;
use crate::registry::ProcessHandle;
use crate::terminals::Unavailable;

/// What the process table proves about one owned process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Liveness {
    /// The PID still carries the owned start token and the process runs.
    Current,
    /// The PID now carries a different start token: another process reused it.
    Replaced,
    /// The process exited: its PID is gone or its record shows it ended.
    Exited,
    /// The table could not prove either way; this is never treated as an exit.
    Unknown(Unavailable),
}

impl Liveness {
    /// Whether the owned process is proven no longer to run.
    #[must_use]
    pub const fn ended(self) -> bool {
        matches!(self, Self::Replaced | Self::Exited)
    }
}

/// Reads `owned`'s PID once and compares its start token.
pub fn liveness(table: &dyn ProcessTable, owned: &ProcessHandle) -> Liveness {
    match table.read(owned.pid()) {
        Ok(native) if native.process.start_token() != owned.start_token() => Liveness::Replaced,
        Ok(native) if native.running => Liveness::Current,
        Ok(_) | Err(Unavailable::ProcessUnavailable) => Liveness::Exited,
        Err(reason) => Liveness::Unknown(reason),
    }
}
