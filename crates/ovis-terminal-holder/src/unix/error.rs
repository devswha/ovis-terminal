//! Typed failures of holder control requests.

use std::io;

use thiserror::Error;

use super::launch_error::{LaunchError, PtySourceError};
use super::link::LinkClosed;
use crate::process::SignalError;
use crate::protocol::{AdoptionError, TerminalId};

/// A holder control request was rejected or could not take effect.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HolderError {
    /// No retained record carries this terminal identity.
    #[error("terminal {terminal_id:?} is not held")]
    UnknownTerminal {
        /// Requested terminal.
        terminal_id: TerminalId,
    },
    /// The terminal or process incarnation does not match the held record.
    #[error("terminal {terminal_id:?} is held with a different incarnation")]
    StaleTarget {
        /// Requested terminal.
        terminal_id: TerminalId,
    },
    /// A spawn named a terminal identity the holder already retains.
    #[error("terminal {terminal_id:?} is already held")]
    DuplicateTerminal {
        /// Requested terminal.
        terminal_id: TerminalId,
    },
    /// The holder is draining and accepts no new terminals.
    #[error("the holder is draining")]
    Draining,
    /// The program could not be launched; no terminal record was kept.
    #[error("the program could not be launched")]
    Launch {
        /// Launch failure.
        #[from]
        source: LaunchError,
    },
    /// Accepting the input would exceed the terminal's input queue bound.
    #[error("{requested} input bytes do not fit: {queued} of {limit} are already queued")]
    InputOverflow {
        /// Bytes already waiting for the PTY.
        queued: usize,
        /// Bytes in the rejected input.
        requested: usize,
        /// Per-terminal queue bound.
        limit: usize,
    },
    /// The input sequence does not advance past the last admitted sequence.
    #[error("input sequence {sequence} does not follow {last}")]
    SequenceRegression {
        /// Rejected sequence.
        sequence: u64,
        /// Last admitted sequence.
        last: u64,
    },
    /// The terminal's process has exited; only its retained record remains.
    #[error("the terminal's process has exited")]
    ProcessExited,
    /// A requested geometry has a zero dimension.
    #[error("geometry {cols}x{rows} has a zero dimension")]
    InvalidGeometry {
        /// Requested columns.
        cols: u16,
        /// Requested rows.
        rows: u16,
    },
    /// The geometry epoch is older than the last applied one.
    #[error("geometry epoch {epoch} is older than applied epoch {applied}")]
    StaleGeometry {
        /// Rejected epoch.
        epoch: u64,
        /// Last applied epoch.
        applied: u64,
    },
    /// The PTY rejected the new geometry.
    #[error("the PTY rejected the new geometry")]
    Resize {
        /// PTY failure.
        #[source]
        source: PtySourceError,
    },
    /// The signal was not delivered.
    #[error("the signal was not delivered")]
    Signal {
        /// Delivery failure.
        #[from]
        source: SignalError,
    },
    /// The requested output offset is beyond the produced output.
    #[error("offset {requested} is beyond output head {head}")]
    OffsetBeyondHead {
        /// Requested offset.
        requested: u64,
        /// Current output head.
        head: u64,
    },
    /// A holder counter or offset cannot advance further.
    #[error("a holder counter is exhausted")]
    CounterExhausted,
    /// Retained exit evidence rejected the request.
    #[error("retained terminal evidence rejected the request")]
    Adoption {
        /// Adoption-table failure.
        #[from]
        source: AdoptionError,
    },
    /// The core link refused the adoption welcome.
    #[error("the core link closed before the welcome was queued ({reason:?})")]
    LinkClosed {
        /// Reason the link closed.
        reason: LinkClosed,
    },
    /// A worker thread could not be started; the process was stopped.
    #[error("a terminal worker thread could not be started")]
    WorkerUnavailable {
        /// Thread creation failure.
        #[source]
        source: io::Error,
        /// Failure while stopping the started process, if any.
        cleanup: Option<SignalError>,
    },
    /// Worker threads panicked before they could be joined.
    #[error("{count} worker thread(s) panicked")]
    WorkerPanicked {
        /// Number of panicked workers.
        count: usize,
    },
}

impl HolderError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnknownTerminal { .. } => "unknown_terminal",
            Self::StaleTarget { .. } => "stale_target",
            Self::DuplicateTerminal { .. } => "duplicate_terminal",
            Self::Draining => "holder_draining",
            Self::Launch { .. } => "spawn_failed",
            Self::InputOverflow { .. } => "input_overflow",
            Self::SequenceRegression { .. } => "input_sequence_regression",
            Self::ProcessExited => "process_exited",
            Self::InvalidGeometry { .. } => "invalid_geometry",
            Self::StaleGeometry { .. } => "stale_geometry",
            Self::Resize { .. } => "resize_failed",
            Self::Signal { .. } => "signal_failed",
            Self::OffsetBeyondHead { .. } => "offset_beyond_head",
            Self::CounterExhausted => "counter_exhausted",
            Self::Adoption { source } => source.kind(),
            Self::LinkClosed { .. } => "link_closed",
            Self::WorkerUnavailable { .. } => "worker_unavailable",
            Self::WorkerPanicked { .. } => "worker_panicked",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::UnknownTerminal { .. } | Self::StaleTarget { .. } => {
                "Re-adopt the holder and use the identities from its welcome."
            }
            Self::DuplicateTerminal { .. } => "Allocate a fresh terminal identity for the launch.",
            Self::Draining => "Launch on the holder of the current build.",
            Self::Launch { .. } => "Inspect the launch error kind and correct the request.",
            Self::InputOverflow { .. } => {
                "Wait for input acknowledgements before sending more bytes."
            }
            Self::SequenceRegression { .. } => "Send input with the next sequence number.",
            Self::ProcessExited => "Read the exit record; the process can no longer be driven.",
            Self::InvalidGeometry { .. } => "Request at least one column and one row.",
            Self::StaleGeometry { .. } => "Resize with the current geometry epoch.",
            Self::Resize { .. } => "Re-read the terminal record and retry the resize.",
            Self::Signal { .. } => "Re-read the terminal record; the process may have exited.",
            Self::OffsetBeyondHead { .. } => "Request an offset at or below the output head.",
            Self::CounterExhausted => "Drain this holder and launch new terminals elsewhere.",
            Self::Adoption { source } => source.hint(),
            Self::LinkClosed { .. } => "Raise the link bound or adopt with fewer retained records.",
            Self::WorkerUnavailable { .. } => "Free thread or memory resources and relaunch.",
            Self::WorkerPanicked { .. } => "Report the holder defect; retained records are intact.",
        }
    }
}
