//! The terminal50 boundary error.

use ovis_terminal_probes::platform::ProbeError;
use ovis_terminal_server::streams::scheduler::{HistoryError, SchedulerError};
use ovis_terminal_vt::VtError;
use thiserror::Error;

type Source = Box<dyn std::error::Error + Send + Sync>;

/// Every failure a terminal50 run can report.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Terminal50Error {
    /// The arguments do not describe one run.
    #[error("the arguments are rejected: {reason}")]
    Usage {
        /// Which rule the arguments broke.
        reason: &'static str,
    },
    /// The run was not asked to use a private root.
    #[error("the run needs a private session root")]
    NotIsolated,
    /// The `--inject` payload names no known fixture.
    #[error("the injected fixture is not one this benchmark plans")]
    Inject {
        /// The decoder's error.
        #[source]
        source: serde_json::Error,
    },
    /// The binary under test cannot be read.
    #[error("the binary under test cannot be read")]
    Binary {
        /// The read error.
        #[source]
        source: std::io::Error,
    },
    /// A terminal produced no output before the readiness bound.
    #[error("terminal {terminal} produced no output within {bound_ms} ms")]
    NotReady {
        /// The silent terminal.
        terminal: usize,
        /// The bound waited.
        bound_ms: u64,
    },
    /// The run finished and broke at least one budget; the report lists each one.
    #[error("{violations} budget checks failed")]
    BudgetExceeded {
        /// Failed checks.
        violations: usize,
    },
    /// CPU time of a process could not be read.
    #[error("the CPU time of process {pid} could not be read")]
    Cpu {
        /// The process.
        pid: u32,
        /// The read error.
        #[source]
        source: Source,
    },
    /// The pseudo-terminal backend refused a step.
    #[error("the pseudo-terminal backend failed to {stage}")]
    Pty {
        /// What was attempted.
        stage: &'static str,
        /// The backend's error.
        #[source]
        source: Source,
    },
    /// Reading, writing or starting a thread failed.
    #[error("an I/O step failed")]
    Io(#[from] std::io::Error),
    /// The scheduler refused or failed.
    #[error("the scheduler failed")]
    Scheduler(#[from] SchedulerError),
    /// A disk history could not be opened.
    #[error("a disk history could not be opened")]
    History(#[from] HistoryError),
    /// A memory probe failed.
    #[error("a platform probe failed")]
    Probe(#[from] ProbeError),
    /// The screen geometry is invalid.
    #[error("the screen geometry is invalid")]
    Geometry(#[from] VtError),
    /// The report did not encode.
    #[error("the report did not encode")]
    Encode(#[from] serde_json::Error),
}

impl Terminal50Error {
    pub(super) fn pty<E: Into<Source>>(stage: &'static str) -> impl FnOnce(E) -> Self {
        move |error| Self::Pty {
            stage,
            source: error.into(),
        }
    }

    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::NotIsolated => "isolation_required",
            Self::Inject { .. } => "invalid_inject",
            Self::Binary { .. } => "binary_unreadable",
            Self::NotReady { .. } => "not_ready",
            Self::BudgetExceeded { .. } => "budget_exceeded",
            Self::Cpu { .. } => "cpu_unreadable",
            Self::Pty { .. } => "pty_failed",
            Self::Io(_) => "io_failed",
            Self::Scheduler(source) => source.kind(),
            Self::History(source) => source.kind(),
            Self::Probe(source) => source.kind(),
            Self::Geometry(_) => "invalid_geometry",
            Self::Encode(_) => "encode_failed",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } => {
                "pass --binary, --isolated, the shape counts and exactly one budget pair"
            }
            Self::NotIsolated => "pass --isolated; the benchmark never uses a live session",
            Self::Inject { .. } => {
                "inject {\"fixture\":\"control-writer-delay\",\"delay_ms\":<ms>}"
            }
            Self::Binary { .. } => "build target/release/ovis-terminal first",
            Self::NotReady { .. } => "check that the host can run PTY programs",
            Self::BudgetExceeded { .. } => "read verdict.violations in the evidence",
            Self::Cpu { .. } => "check /proc or ps on this host",
            Self::Pty { .. } => "check that the host can open pseudo-terminals",
            Self::Io(_) | Self::Encode(_) => "check the evidence path and the process limits",
            Self::Scheduler(source) => source.hint(),
            Self::History(source) => source.hint(),
            Self::Probe(source) => source.hint(),
            Self::Geometry(_) => "use at least the minimum column count",
        }
    }

    /// Process exit status: 2 for arguments, 1 for every failed run.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage { .. } | Self::NotIsolated | Self::Inject { .. } => 2,
            _ => 1,
        }
    }
}
