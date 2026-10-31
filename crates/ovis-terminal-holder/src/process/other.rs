//! Other Unix systems: the kernel start time through `ps -o lstart=` (one-second
//! resolution); every process-table field is reported as unsupported.

use std::path::PathBuf;

use super::table::{ProbeError, ProcessEntry};
use super::{ProcessId, StartTokenError};
use crate::protocol::ProcessStartToken;

pub(super) fn start_token(pid: ProcessId) -> Result<ProcessStartToken, StartTokenError> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "lstart=", "-p", &pid.get().to_string()])
        .env("LC_ALL", "C")
        .output()
        .map_err(|source| StartTokenError::Unreadable { source })?;
    if !output.status.success() {
        return Err(StartTokenError::ProcessGone { pid: pid.get() });
    }
    let text = String::from_utf8(output.stdout).map_err(|_| StartTokenError::Malformed)?;
    let started = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if started.is_empty() {
        return Err(StartTokenError::Malformed);
    }
    Ok(ProcessStartToken::new(format!("lstart:{started}")))
}

/// Reads the record of `pid`.
///
/// # Errors
///
/// Returns [`ProbeError::Gone`] when no process has the PID, or a typed read failure.
pub const fn read_entry(_: ProcessId) -> Result<ProcessEntry, ProbeError> {
    Err(ProbeError::Unsupported)
}

/// Lists the PIDs that may have `tty` as their controlling terminal; callers re-check
/// each record's membership.
///
/// # Errors
///
/// Returns [`ProbeError::Unsupported`] where the table cannot be enumerated.
pub const fn list_terminal_pids(_: u64) -> Result<Vec<ProcessId>, ProbeError> {
    Err(ProbeError::Unsupported)
}

/// Reads the current argument vector of `pid`.
///
/// # Errors
///
/// Returns [`ProbeError::Malformed`] for a non-UTF-8 argument, or a typed read failure.
pub const fn read_argv(_: ProcessId) -> Result<Vec<String>, ProbeError> {
    Err(ProbeError::Unsupported)
}

/// Reads the current working directory of `pid`.
///
/// # Errors
///
/// Returns a typed read failure, or [`ProbeError::Unsupported`] where unavailable.
pub const fn read_cwd(_: ProcessId) -> Result<PathBuf, ProbeError> {
    Err(ProbeError::Unsupported)
}
