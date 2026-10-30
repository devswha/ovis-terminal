//! Operating-system start tokens that distinguish a process from a reused PID.
//!
//! Linux reads the kernel start time (clock ticks since boot) from
//! `/proc/<pid>/stat` and binds it to the current boot ID. Other Unix systems
//! read the kernel start time through `ps -o lstart=` (one-second resolution);
//! plan todo 55 owns cross-platform consistency of this evidence.

use std::io;

use thiserror::Error;

use super::ProcessId;
use crate::protocol::ProcessStartToken;

/// Failure to read a start token.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StartTokenError {
    /// The process no longer exists.
    #[error("process {pid} no longer exists")]
    ProcessGone {
        /// PID that was looked up.
        pid: u32,
    },
    /// The operating-system record could not be read.
    #[error("process start record could not be read")]
    Unreadable {
        /// Underlying read failure.
        #[source]
        source: io::Error,
    },
    /// The operating-system record did not have the expected layout.
    #[error("process start record has an unexpected layout")]
    Malformed,
}

impl StartTokenError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ProcessGone { .. } => "process_gone",
            Self::Unreadable { .. } => "start_token_unreadable",
            Self::Malformed => "start_token_malformed",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::ProcessGone { .. } => "Treat the process as exited; do not reuse its PID.",
            Self::Unreadable { .. } => "Check that the process table is readable by the holder.",
            Self::Malformed => "Report the platform; its process record format is unsupported.",
        }
    }
}

/// Reads the start token of a live process.
///
/// # Errors
///
/// Returns [`StartTokenError::ProcessGone`] when the PID names no process, and
/// a read or layout error when the operating-system record is unusable.
pub fn read_start_token(pid: ProcessId) -> Result<ProcessStartToken, StartTokenError> {
    os_start_token(pid).map(ProcessStartToken::new)
}

#[cfg(target_os = "linux")]
fn os_start_token(pid: ProcessId) -> Result<String, StartTokenError> {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.get())).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            StartTokenError::ProcessGone { pid: pid.get() }
        } else {
            StartTokenError::Unreadable { source }
        }
    })?;
    // The command name may contain spaces or parentheses; fields resume after the last ')'.
    // `starttime` is field 22 overall, the 20th field after the name.
    let (_, fields) = stat.rsplit_once(')').ok_or(StartTokenError::Malformed)?;
    let start_ticks = fields
        .split_whitespace()
        .nth(19)
        .and_then(|raw| raw.parse::<u64>().ok())
        .ok_or(StartTokenError::Malformed)?;
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|source| StartTokenError::Unreadable { source })?;
    Ok(format!("linux:{}:{start_ticks}", boot_id.trim()))
}

#[cfg(not(target_os = "linux"))]
fn os_start_token(pid: ProcessId) -> Result<String, StartTokenError> {
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
    Ok(format!("lstart:{started}"))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{ProcessId, read_start_token};

    #[test]
    fn token_is_stable_while_the_process_lives() {
        let own = ProcessId::new(std::process::id());

        let first = read_start_token(own).expect("own token");
        let second = read_start_token(own).expect("own token again");

        assert_eq!(first, second);
    }

    #[test]
    fn reaped_process_reports_process_gone() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn child");
        let pid = ProcessId::new(child.id());
        assert!(child.wait().expect("child exit").success());

        let error = read_start_token(pid).expect_err("reaped pid has no token");

        assert_eq!(error.kind(), "process_gone");
    }
}
