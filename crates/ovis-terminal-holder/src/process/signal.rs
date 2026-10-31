//! Signal delivery to holder-owned processes.
//!
//! Delivery is a fact on its own: a successful call means the kernel accepted
//! the signal, never that the process stopped. Termination evidence is only the
//! holder's later exit record (EXCL-026).

use std::io;

use thiserror::Error;

use super::ProcessId;
use crate::protocol::SignalKind;

/// Which processes a signal reaches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SignalScope {
    /// Only the process with this PID.
    Process(ProcessId),
    /// Every member of the process group led by this PID.
    ProcessGroup(ProcessId),
}

/// Signal delivery failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SignalError {
    /// The PID cannot be expressed as a positive `pid_t`.
    #[error("process identifier {pid} cannot be addressed by the operating system")]
    PidOutOfRange {
        /// PID that could not be converted.
        pid: u32,
    },
    /// The kernel refused or could not deliver the signal.
    #[error("the operating system did not accept the signal")]
    Delivery {
        /// Error reported by `kill(2)`.
        #[source]
        source: io::Error,
    },
}

impl SignalError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::PidOutOfRange { .. } => "pid_out_of_range",
            Self::Delivery { .. } => "signal_not_delivered",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::PidOutOfRange { .. } => "Address only PIDs reported by the holder.",
            Self::Delivery { .. } => {
                "Re-read the terminal record; the process may already have exited."
            }
        }
    }
}

/// Sends `signal` to the processes in `scope`.
///
/// # Errors
///
/// Returns [`SignalError::PidOutOfRange`] for a PID that is not a positive
/// `pid_t`, and [`SignalError::Delivery`] when `kill(2)` fails.
pub fn send_signal(scope: SignalScope, signal: SignalKind) -> Result<(), SignalError> {
    let (pid, group) = match scope {
        SignalScope::Process(pid) => (pid, false),
        SignalScope::ProcessGroup(pid) => (pid, true),
    };
    let raw = libc::pid_t::try_from(pid.get())
        .ok()
        .filter(|raw| *raw > 0)
        .ok_or_else(|| SignalError::PidOutOfRange { pid: pid.get() })?;
    // A negative PID addresses the whole process group; `raw > 0`, so negation cannot overflow.
    let addressed = if group { raw.saturating_neg() } else { raw };
    let number = match signal {
        SignalKind::Interrupt => libc::SIGINT,
        SignalKind::Terminate => libc::SIGTERM,
        SignalKind::Kill => libc::SIGKILL,
    };
    // SAFETY: kill(2) takes two integers by value and neither reads nor writes
    // memory owned by this process.
    let status = unsafe { libc::kill(addressed, number) };
    if status == 0 {
        Ok(())
    } else {
        Err(SignalError::Delivery {
            source: io::Error::last_os_error(),
        })
    }
}
