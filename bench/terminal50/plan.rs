//! What one run measures and the budgets it is judged against.
//!
//! The IDEAL profile (IDEAL-027, IDEAL-051) has one fixed shape and is judged on every
//! sample; the capacity profile is judged on p99 and is labelled a release target, never
//! the IDEAL assumption.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The exact shape of the IDEAL latency profile.
pub(super) const IDEAL_SHAPE: Shape = Shape {
    terminals: 16,
    clients: 8,
    inflight: 16,
    bytes_per_second: 1_048_576,
};

/// The exact shape of the 50-terminal capacity release target.
pub(super) const CAPACITY_SHAPE: Shape = Shape {
    terminals: 50,
    ..IDEAL_SHAPE
};

/// How much load a run applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Shape {
    /// Terminals, each with one real PTY program.
    pub(super) terminals: usize,
    /// Control-event clients, each subscribed to every terminal.
    pub(super) clients: usize,
    /// Request lanes; each keeps one request in flight.
    pub(super) inflight: usize,
    /// Aggregate program output across all terminals.
    pub(super) bytes_per_second: usize,
}

/// Which budgets the run is judged against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "profile", rename_all = "snake_case")]
pub(super) enum Profile {
    /// IDEAL-027/IDEAL-051: every admission and every event sample within its bound.
    Ideal {
        admission_max_ms: u64,
        event_max_ms: u64,
    },
    /// The 50-terminal capacity release target: p99 within its bound.
    Capacity {
        admission_p99_ms: u64,
        event_p99_ms: u64,
    },
}

/// A deliberate defect a failure run injects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "fixture", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum Fixture {
    /// Every control-event writer waits `delay_ms` before each write.
    ControlWriterDelay { delay_ms: u64 },
    /// Every admission waits `delay_ms` between its reservation and its admit.
    AdmissionDelay { delay_ms: u64 },
}

/// Where each terminal's output comes from.
#[derive(Debug, Clone)]
pub(super) enum Source {
    /// A real PTY running `exe emit <rate>`.
    Pty { exe: PathBuf },
    /// A paced in-process feed: no child process, for the crate's own tests.
    #[cfg(test)]
    InProcess,
}

/// One validated run.
#[derive(Debug, Clone)]
pub(super) struct Plan {
    pub(super) shape: Shape,
    pub(super) profile: Profile,
    /// Measured samples wanted after the warmup.
    pub(super) samples: usize,
    /// Leading samples reported separately as warmup.
    pub(super) warmup: usize,
    /// Output viewers per terminal; the first never reads.
    pub(super) viewers: usize,
    pub(super) rss_limit_bytes: u64,
    pub(super) disk_limit_bytes: u64,
    pub(super) workers: usize,
    /// Hang guard on the whole measurement.
    pub(super) max_run: Duration,
    /// Hang guard on draining event writers after the load stops.
    pub(super) drain: Duration,
    /// Bound on each teardown step.
    pub(super) teardown: Duration,
    pub(super) fixture: Option<Fixture>,
    pub(super) source: Source,
    /// The private directory every file of the run lives under.
    pub(super) root: PathBuf,
}

impl Plan {
    pub(super) const fn label(&self) -> &'static str {
        match self.profile {
            Profile::Ideal { .. } => "ideal-16pty",
            Profile::Capacity { .. } => "capacity-release-target",
        }
    }

    /// Output bytes per second of one terminal.
    pub(super) fn per_terminal_rate(&self) -> usize {
        self.shape
            .bytes_per_second
            .checked_div(self.shape.terminals)
            .unwrap_or(0)
    }

    pub(super) const fn writer_delay(&self) -> Option<Duration> {
        match self.fixture {
            Some(Fixture::ControlWriterDelay { delay_ms }) => Some(Duration::from_millis(delay_ms)),
            _ => None,
        }
    }

    pub(super) const fn admission_delay(&self) -> Option<Duration> {
        match self.fixture {
            Some(Fixture::AdmissionDelay { delay_ms }) => Some(Duration::from_millis(delay_ms)),
            _ => None,
        }
    }
}
