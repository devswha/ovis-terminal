//! Native process-table records: PID, start token, run state and terminal membership.
//!
//! Linux reads `/proc`, macOS reads `proc_pidinfo`, Windows reads the process object's
//! creation time. A field an operating system cannot prove is reported as
//! [`ProbeError::Unsupported`] or [`Membership::Unsupported`], never guessed.

use std::io;

use thiserror::Error;

use super::{ProcessId, StartTokenError};
use crate::protocol::ProcessStartToken;

/// The controlling-terminal facts of one process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TerminalMembership {
    /// Session of the process. A process whose terminal was closed can keep naming a
    /// terminal device number that a newer terminal reuses, so membership is the pair of
    /// session and terminal, never the device number alone.
    pub session: u32,
    /// Process group of the process.
    pub group: u32,
    /// Device number of its controlling terminal.
    pub tty: u64,
    /// Foreground process group of that terminal.
    pub foreground_group: u32,
}

/// Whether and how a process belongs to a controlling terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Membership {
    /// The process has this controlling terminal.
    Controlling(TerminalMembership),
    /// The process has no controlling terminal.
    Detached,
    /// The operating system does not expose terminal membership.
    Unsupported,
}

/// One process-table record, read in a single native call where the OS allows.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ProcessEntry {
    /// Process identifier.
    pub pid: ProcessId,
    /// Start token read with the record.
    pub start_token: ProcessStartToken,
    /// False for an exited process whose record still exists (a zombie, or a
    /// Windows process object kept open by a handle).
    pub running: bool,
    /// Controlling-terminal membership.
    pub terminal: Membership,
}

/// A process-table read that produced no record.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProbeError {
    /// No process has this PID.
    #[error("process {pid} no longer exists")]
    Gone {
        /// PID that was looked up.
        pid: u32,
    },
    /// The operating system refused the read.
    #[error("the process record is not readable by this user")]
    Permission {
        /// Refusal reported by the operating system.
        #[source]
        source: io::Error,
    },
    /// This operating system does not expose the requested record.
    #[error("the process record is not available on this platform")]
    Unsupported,
    /// The record did not have the expected layout or encoding.
    #[error("the process record has an unexpected layout or encoding")]
    Malformed,
    /// The record could not be read for another reason.
    #[error("the process record could not be read")]
    Unreadable {
        /// Underlying read failure.
        #[source]
        source: io::Error,
    },
}

impl ProbeError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Gone { .. } => "process_gone",
            Self::Permission { .. } => "permission_denied",
            Self::Unsupported => "unsupported_platform",
            Self::Malformed => "process_record_malformed",
            Self::Unreadable { .. } => "process_record_unreadable",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Gone { .. } => "Treat the process as ended; never reuse its PID alone.",
            Self::Permission { .. } => "Run the holder as the user that owns the process.",
            Self::Unsupported => "Report the field as unsupported on this platform.",
            Self::Malformed => "Report the platform; its process record format is unsupported.",
            Self::Unreadable { .. } => "Read the record again; the process table changed.",
        }
    }
}

impl From<ProbeError> for StartTokenError {
    fn from(error: ProbeError) -> Self {
        match error {
            ProbeError::Gone { pid } => Self::ProcessGone { pid },
            ProbeError::Permission { source } | ProbeError::Unreadable { source } => {
                Self::Unreadable { source }
            }
            ProbeError::Unsupported => Self::Unreadable {
                source: io::Error::from(io::ErrorKind::Unsupported),
            },
            ProbeError::Malformed => Self::Malformed,
        }
    }
}

#[cfg(test)]
mod tests;
