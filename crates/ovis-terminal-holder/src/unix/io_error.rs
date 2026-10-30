//! Typed failures of a terminal's PTY pump, writer and exit waiter.
//!
//! A PTY hang-up (EOF, EIO, POLLHUP) is the normal end of a terminal and is not
//! an error. Anything else keeps its source here and reaches the core as typed
//! exit evidence instead of looking like an ordinary end of output or exit.

use std::io;

use thiserror::Error;

use super::error::HolderError;
use crate::protocol::{ExitUnknown, ExitUnknownReason, PtyIoFailure, PtyOperation};

/// An unexpected terminal I/O or process-observation failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TerminalIoError {
    /// Reading program output failed.
    #[error("reading program output failed")]
    Read {
        /// Read failure.
        #[source]
        source: io::Error,
    },
    /// Waiting for the PTY to become ready failed.
    #[error("waiting for the pseudo-terminal failed")]
    Poll {
        /// Poll failure.
        #[source]
        source: io::Error,
    },
    /// Writing program input failed; the acknowledgement carries the written prefix.
    #[error("writing program input failed")]
    Write {
        /// Write failure.
        #[source]
        source: io::Error,
    },
    /// Output could not be recorded into the retained tail.
    #[error("program output could not be recorded")]
    Record {
        /// Recording failure.
        #[source]
        source: HolderError,
    },
    /// The output pump ended before it could record the exit.
    #[error("the output pump ended without recording the exit")]
    PumpLost,
    /// Waiting for the process failed; its exit status is unknown.
    #[error("waiting for the process failed")]
    Wait {
        /// Wait failure.
        #[source]
        source: io::Error,
    },
    /// The process waiter ended without reporting an exit status.
    #[error("the process waiter ended without reporting")]
    WaiterLost,
}

impl TerminalIoError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Read { .. } => "pty_read_failed",
            Self::Poll { .. } => "pty_poll_failed",
            Self::Write { .. } => "pty_write_failed",
            Self::Record { .. } => "output_record_failed",
            Self::PumpLost => "pump_lost",
            Self::Wait { .. } => "wait_failed",
            Self::WaiterLost => "waiter_lost",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Read { .. } | Self::Poll { .. } | Self::Record { .. } | Self::PumpLost => {
                "Treat output after the final offset as lost; read the retained tail."
            }
            Self::Write { .. } => "Resend unwritten input only after re-reading the terminal.",
            Self::Wait { .. } | Self::WaiterLost => {
                "Treat the exit status as unknown; do not infer success."
            }
        }
    }

    /// Wire form for an output or input failure; `None` for exit-status failures.
    pub(super) fn io_failure(&self) -> Option<PtyIoFailure> {
        let (operation, source) = match self {
            Self::Read { source } => (PtyOperation::Read, Some(source)),
            Self::Poll { source } => (PtyOperation::Poll, Some(source)),
            Self::Write { source } => (PtyOperation::Write, Some(source)),
            Self::Record { .. } => (PtyOperation::Record, None),
            Self::PumpLost => (PtyOperation::Pump, None),
            Self::Wait { .. } | Self::WaiterLost => return None,
        };
        let os_error = source.and_then(io::Error::raw_os_error);
        Some(PtyIoFailure {
            operation,
            os_error,
        })
    }

    /// Wire form for an unknown exit status; `None` for I/O failures.
    pub(super) fn exit_unknown(&self) -> Option<ExitUnknown> {
        let (reason, os_error) = match self {
            Self::Wait { source } => (ExitUnknownReason::WaitFailed, source.raw_os_error()),
            Self::WaiterLost => (ExitUnknownReason::WaiterLost, None),
            Self::Read { .. }
            | Self::Poll { .. }
            | Self::Write { .. }
            | Self::Record { .. }
            | Self::PumpLost => return None,
        };
        Some(ExitUnknown { reason, os_error })
    }
}
