//! The wait boundary error.

use ovis_terminal_protocol::schema::scalar::{ReportSeq, TurnNumber};
use ovis_terminal_vt::OutputOffset;

use super::facts::ProcessState;
use crate::screen::ScreenError;

/// Every rejection of a wait registration or a fact fed to the hub.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WaitError {
    /// The cursor lies before the oldest retained output byte.
    #[error("output from byte {requested} was evicted; retained output starts at byte {floor}")]
    OutputGap {
        /// Cursor the caller presented.
        requested: OutputOffset,
        /// Oldest retained byte.
        floor: OutputOffset,
        /// Offset after the newest byte.
        head: OutputOffset,
    },
    /// The cursor lies after the newest output byte.
    #[error("cursor {cursor} lies after the newest output byte {head}")]
    CursorAhead {
        /// Cursor the caller presented.
        cursor: OutputOffset,
        /// Offset after the newest byte.
        head: OutputOffset,
    },
    /// The pattern is empty and would match before any output.
    #[error("the output pattern is empty")]
    EmptyPattern,
    /// The state wait names no state and no exit.
    #[error("the state wait names no state and does not wait for exit")]
    EmptyCondition,
    /// The turn is older than the retained turns or never started.
    #[error("turn {} is not retained", .turn.get())]
    UnknownTurn {
        /// Turn the caller named.
        turn: TurnNumber,
    },
    /// A status report does not advance the producer sequence.
    #[error(
        "report sequence {} does not exceed the current sequence {}",
        .received.get(),
        .current.get()
    )]
    StaleReport {
        /// Sequence of the rejected report.
        received: ReportSeq,
        /// Sequence already applied.
        current: ReportSeq,
    },
    /// A new turn does not advance the turn number.
    #[error("turn {} does not follow turn {}", .received.get(), .current.get())]
    StaleTurn {
        /// Turn the rejected report starts.
        received: TurnNumber,
        /// Newest turn already started.
        current: TurnNumber,
    },
    /// Status was reported for a process that is no longer running.
    #[error("the process is no longer running")]
    NotRunning {
        /// What the holder reported about the process.
        process: ProcessState,
    },
    /// The screen refused the holder output or exit.
    #[error("the screen refused the holder evidence")]
    Screen(#[source] ScreenError),
    /// An observer resolved without an outcome.
    #[error("the observer left the table without an outcome")]
    Detached,
    /// A thread panicked while holding the wait table.
    #[error("the wait table lock was poisoned by a panicking thread")]
    Poisoned,
}

impl WaitError {
    /// Stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::OutputGap { .. } => "output_gap",
            Self::CursorAhead { .. } | Self::EmptyCondition => "invalid_params",
            Self::EmptyPattern => "invalid_pattern",
            Self::UnknownTurn { .. } => "unknown_turn",
            Self::StaleReport { .. } | Self::StaleTurn { .. } => "stale_seq",
            Self::NotRunning { .. } => "finished",
            Self::Screen(error) => error.kind(),
            Self::Detached => "wait_detached",
            Self::Poisoned => "waits_poisoned",
        }
    }

    /// Recovery hint for the caller.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::OutputGap { .. } => {
                "read the retained range with terminal.output, then wait from a fresh cursor"
            }
            Self::CursorAhead { .. } => "use a cursor returned by a receipt or a previous wait",
            Self::EmptyPattern => "wait for a non-empty pattern",
            Self::EmptyCondition => "name at least one declared state or wait for exit",
            Self::UnknownTurn { .. } => "wait on the send operation, or read turn.get first",
            Self::StaleReport { .. } => "report with a sequence above the current one",
            Self::StaleTurn { .. } => "start each turn with a number above the previous one",
            Self::NotRunning { .. } => "status ends with the process; report from its successor",
            Self::Screen(error) => error.hint(),
            Self::Detached => "arm the wait again; this is a defect in the hub",
            Self::Poisoned => "rebuild the terminal's wait hub from its screen",
        }
    }
}
