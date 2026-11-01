//! The scheduling boundary error: stable kinds, recovery hints and sources.

use std::fmt;

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use thiserror::Error;

/// Which aggregate a byte budget bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BudgetScope {
    /// One terminal's accepted input queue.
    TerminalInput,
    /// Accepted input queues of every terminal in the session.
    SessionInput,
    /// Control requests queued at every terminal in the session.
    SessionControl,
    /// Output waiting to be parsed at every terminal in the session.
    SessionOutput,
    /// Event retention of every terminal in the session.
    SessionEvents,
}

impl BudgetScope {
    const fn is_input(self) -> bool {
        matches!(self, Self::TerminalInput | Self::SessionInput)
    }
}

impl fmt::Display for BudgetScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TerminalInput => "terminal input",
            Self::SessionInput => "session input",
            Self::SessionControl => "session control",
            Self::SessionOutput => "session output",
            Self::SessionEvents => "session event",
        })
    }
}

/// Why scheduling refused work; a refusal admitted, allocated and wrote nothing.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SchedulerError {
    /// Reserving the units would pass a budget.
    #[error("{offered} more units would pass the {limit}-unit {scope} bound; {held} are held")]
    Exhausted {
        /// The budget that refused.
        scope: BudgetScope,
        /// Units asked for.
        offered: u64,
        /// Units already held.
        held: u64,
        /// The bound.
        limit: u64,
    },
    /// More bytes were supplied than were reserved.
    #[error("{offered} bytes were supplied for a {reserved}-byte reservation")]
    OverReservation {
        /// Bytes reserved.
        reserved: u64,
        /// Bytes supplied.
        offered: u64,
    },
    /// The terminal's bounded admission or control queue is full.
    #[error("the {lane} queue holds its {capacity} entries")]
    QueueFull {
        /// `admission` or `control`.
        lane: &'static str,
        /// Entries the queue holds.
        capacity: usize,
    },
    /// Every bounded blocking slot is busy.
    #[error("all {limit} blocking slots are busy")]
    BlockingSaturated {
        /// Slots in the gate.
        limit: usize,
    },
    /// The database writer's bounded queue is full.
    #[error("the database writer holds its {capacity} queued jobs")]
    WriterBusy {
        /// Jobs the queue holds.
        capacity: usize,
    },
    /// The terminal actor has stopped.
    #[error("the terminal actor has stopped")]
    ActorStopped,
    /// The terminal keeps no disk history.
    #[error("the terminal keeps no disk history")]
    NoHistory,
    /// The disk history refused or failed.
    #[error("the disk history refused the request")]
    History(#[from] super::history::HistoryError),
    /// The database writer has stopped.
    #[error("the database writer has stopped")]
    WriterStopped,
    /// A dedicated thread could not start.
    #[error("a scheduler thread could not start")]
    Spawn(#[source] std::io::Error),
    /// A dedicated thread or task ended by panicking.
    #[error("a scheduler thread ended by panicking")]
    Panicked,
    /// A blocking job did not complete.
    #[error("a blocking job did not complete")]
    Join(#[from] tokio::task::JoinError),
}

impl SchedulerError {
    /// The stable machine-readable kind clients branch on.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Exhausted { scope, .. } if scope.is_input() => "input_overflow",
            Self::Exhausted { .. }
            | Self::QueueFull { .. }
            | Self::BlockingSaturated { .. }
            | Self::WriterBusy { .. } => "backpressure",
            Self::OverReservation { .. } | Self::NoHistory => "invalid_params",
            Self::History(source) => source.kind(),
            Self::ActorStopped | Self::WriterStopped => "unavailable",
            Self::Spawn(_) | Self::Panicked | Self::Join(_) => "internal",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Exhausted { scope, .. } if scope.is_input() => {
                "wait for the program to read queued input, or send fewer bytes at once"
            }
            Self::Exhausted { .. } => "retry once queued work of the session drains",
            Self::OverReservation { .. } => "reserve the exact byte count before admitting it",
            Self::NoHistory => "open the terminal with a history store to read history",
            Self::History(source) => source.hint(),
            Self::QueueFull { .. } | Self::BlockingSaturated { .. } | Self::WriterBusy { .. } => {
                "retry after an outstanding request completes"
            }
            Self::ActorStopped | Self::WriterStopped => "resolve the terminal again; it ended",
            Self::Spawn(_) | Self::Panicked | Self::Join(_) => {
                "restart the control server; terminals keep running"
            }
        }
    }

    const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Exhausted { .. }
                | Self::QueueFull { .. }
                | Self::BlockingSaturated { .. }
                | Self::WriterBusy { .. }
        )
    }

    /// The typed wire error.
    #[must_use]
    pub fn boundary(&self) -> BoundaryError {
        BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), self.retryable())
            .with_hint(self.hint())
    }
}
