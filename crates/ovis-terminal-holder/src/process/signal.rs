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
    /// PID 0 or 1: `kill(2)` would address the caller's own process group,
    /// init, or (as a group) every process the caller may signal.
    #[error("process identifier {pid} is reserved and is never signalled")]
    ReservedPid {
        /// Reserved PID that was refused before any system call.
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
            Self::ReservedPid { .. } => "reserved_pid",
            Self::Delivery { .. } => "signal_not_delivered",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::PidOutOfRange { .. } => "Address only PIDs reported by the holder.",
            Self::ReservedPid { .. } => {
                "Treat the target as unknown; a holder-owned child never has PID 0 or 1."
            }
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
/// Returns [`SignalError::ReservedPid`] for PID 0 or 1 without calling
/// `kill(2)`, [`SignalError::PidOutOfRange`] for a PID that is not a `pid_t`,
/// and [`SignalError::Delivery`] when `kill(2)` fails.
pub fn send_signal(scope: SignalScope, signal: SignalKind) -> Result<(), SignalError> {
    deliver(scope, number(signal), kill)
}

/// Sends SIGHUP to the processes in `scope`, with the same reserved-PID refusal.
///
/// SIGHUP is a local kill stage, not a generation-two `SignalKind`.
///
/// # Errors
///
/// As [`send_signal`].
pub fn send_hangup(scope: SignalScope) -> Result<(), SignalError> {
    deliver(scope, libc::SIGHUP, kill)
}

fn kill(pid: libc::pid_t, number: libc::c_int) -> libc::c_int {
    // SAFETY: kill(2) takes two integers by value and neither reads nor writes
    // memory owned by this process.
    unsafe { libc::kill(pid, number) }
}

const fn number(signal: SignalKind) -> libc::c_int {
    match signal {
        SignalKind::Interrupt => libc::SIGINT,
        SignalKind::Terminate => libc::SIGTERM,
        SignalKind::Kill => libc::SIGKILL,
    }
}

/// Validates the target, then calls `kill` exactly once; it is never reached for
/// a reserved PID, because kill(0)/kill(-1)/killpg(1) would reach far beyond
/// holder-owned children.
fn deliver(
    scope: SignalScope,
    number: libc::c_int,
    kill: impl FnOnce(libc::pid_t, libc::c_int) -> libc::c_int,
) -> Result<(), SignalError> {
    let (pid, group) = match scope {
        SignalScope::Process(pid) => (pid, false),
        SignalScope::ProcessGroup(pid) => (pid, true),
    };
    if pid.get() <= 1 {
        return Err(SignalError::ReservedPid { pid: pid.get() });
    }
    let raw = libc::pid_t::try_from(pid.get())
        .map_err(|_| SignalError::PidOutOfRange { pid: pid.get() })?;
    // A negative PID addresses the whole process group; `raw > 1`, so negation cannot overflow.
    let addressed = if group { raw.saturating_neg() } else { raw };
    if kill(addressed, number) == 0 {
        Ok(())
    } else {
        Err(SignalError::Delivery {
            source: io::Error::last_os_error(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{ProcessId, SignalError, SignalScope, deliver, number};
    use crate::protocol::SignalKind;

    /// Records every requested `kill(2)` call instead of making it.
    fn recorded(
        scope: SignalScope,
        signal: libc::c_int,
    ) -> (Result<(), SignalError>, Vec<(i32, i32)>) {
        let calls = Cell::new(Vec::new());
        let result = deliver(scope, signal, |pid, number| {
            let mut seen = calls.take();
            seen.push((pid, number));
            calls.set(seen);
            0
        });
        (result, calls.take())
    }

    #[test]
    fn reserved_pids_are_refused_before_kill_is_reached() {
        // Given: every scope addressing PID 0 or 1, with every wire signal and SIGHUP.
        for raw in [0, 1] {
            for scope in [
                SignalScope::Process(ProcessId::new(raw)),
                SignalScope::ProcessGroup(ProcessId::new(raw)),
            ] {
                for signal in [
                    number(SignalKind::Interrupt),
                    number(SignalKind::Terminate),
                    number(SignalKind::Kill),
                    libc::SIGHUP,
                ] {
                    // When: delivery is requested.
                    let (result, calls) = recorded(scope, signal);

                    // Then: it is a typed refusal and kill(2) was never called.
                    assert_eq!(calls, [], "{scope:?} reached kill");
                    let error = result.expect_err("reserved pid must be refused");
                    assert!(matches!(error, SignalError::ReservedPid { pid } if pid == raw));
                    assert_eq!(error.kind(), "reserved_pid");
                }
            }
        }
    }

    #[test]
    fn unrepresentable_pid_is_refused_before_kill_is_reached() {
        // Given: a PID beyond pid_t.
        let raw = u32::try_from(libc::pid_t::MAX).expect("positive") + 1;

        // When: a group signal is requested.
        let (result, calls) = recorded(
            SignalScope::ProcessGroup(ProcessId::new(raw)),
            number(SignalKind::Kill),
        );

        // Then: no kill(2) call, typed out-of-range refusal.
        assert_eq!(calls, []);
        assert!(matches!(result, Err(SignalError::PidOutOfRange { pid }) if pid == raw));
    }

    #[test]
    fn ordinary_targets_reach_kill_once_with_group_negation() {
        // Given: the smallest non-reserved PID, as a process and as a group.
        let pid = ProcessId::new(2);

        // When: delivery is requested through the recording seam.
        let (process, process_calls) =
            recorded(SignalScope::Process(pid), number(SignalKind::Terminate));
        let (group, group_calls) =
            recorded(SignalScope::ProcessGroup(pid), number(SignalKind::Kill));

        // Then: exactly one call each, the group addressed by its negative id.
        assert!(process.is_ok() && group.is_ok());
        assert_eq!(process_calls, [(2, libc::SIGTERM)]);
        assert_eq!(group_calls, [(-2, libc::SIGKILL)]);
    }
}
