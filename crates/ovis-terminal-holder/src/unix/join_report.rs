//! What holder shutdown proves about each terminal's worker threads.
//!
//! Every outcome comes from an actual `JoinHandle::join` result; a worker whose
//! handle was never registered is reported as such, never as joined.

use std::collections::BTreeMap;

use crate::protocol::TerminalId;

/// The three threads each terminal owns.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum WorkerRole {
    /// Waits for the process exit and the drain barrier.
    Waiter,
    /// Reads PTY output and records the exit.
    Pump,
    /// Writes queued input to the PTY.
    Writer,
}

impl WorkerRole {
    pub(super) const ALL: [Self; 3] = [Self::Waiter, Self::Pump, Self::Writer];
}

/// Result of joining one worker thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum JoinOutcome {
    /// `join` returned after the thread's closure finished.
    Joined,
    /// No handle was registered for this worker, so it was never joined.
    NotRegistered,
    /// `join` reported that the thread panicked.
    Panicked,
}

/// Join outcomes of one terminal's workers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct WorkersJoined {
    /// Exit waiter.
    pub waiter: JoinOutcome,
    /// Output pump.
    pub pump: JoinOutcome,
    /// Input writer.
    pub writer: JoinOutcome,
}

impl WorkersJoined {
    /// Reports whether all three workers were joined without panicking.
    #[must_use]
    pub fn all_joined(&self) -> bool {
        [self.waiter, self.pump, self.writer]
            .iter()
            .all(|outcome| *outcome == JoinOutcome::Joined)
    }
}

/// Join outcomes for every terminal whose workers were started in this holder.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// Retained targets already exited when shutdown considered signalling them.
    pub already_exited: usize,
    /// Outcomes by terminal identity.
    pub terminals: BTreeMap<TerminalId, WorkersJoined>,
}

impl ShutdownReport {
    /// Number of workers that panicked.
    #[must_use]
    pub fn panicked(&self) -> usize {
        self.terminals
            .values()
            .flat_map(|joined| [joined.waiter, joined.pump, joined.writer])
            .filter(|outcome| *outcome == JoinOutcome::Panicked)
            .count()
    }
}
