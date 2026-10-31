//! The tuning harness boundary error.

use ovis_terminal_probes::platform::ProbeError;
use ovis_terminal_server::streams::scheduler::{HistoryError, SchedulerError};
use ovis_terminal_vt::VtError;
use thiserror::Error;

type Source = Box<dyn std::error::Error + Send + Sync>;

/// Every failure a tuning run can report.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TuningError {
    /// The arguments name no mode.
    #[error(
        "expected `parse`, `load <terminals> <viewers> <bytes-per-second> <seconds>` or `emit <bytes-per-second> <seconds>`"
    )]
    Usage,
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

impl TuningError {
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
            Self::Usage => "usage",
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
            Self::Usage => "pass a mode and its counts",
            Self::Pty { .. } => "check that the host can open pseudo-terminals",
            Self::Io(_) | Self::Encode(_) => "check stdout and the process limits",
            Self::Scheduler(source) => source.hint(),
            Self::History(source) => source.hint(),
            Self::Probe(source) => source.hint(),
            Self::Geometry(_) => "use at least the minimum column count",
        }
    }
}
