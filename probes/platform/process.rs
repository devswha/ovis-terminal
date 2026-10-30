//! Process identity evidence: PIDs, start tokens, liveness and resident memory.

use std::fmt;

use ovis_terminal_holder::protocol::ProcessStartToken;
use serde::{Deserialize, Serialize};

use super::error::{Capability, ProbeError};
use super::measure::ByteCount;

/// An operating-system process identifier.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "PidText", into = "PidText")]
pub struct Pid(u32);

#[derive(Deserialize, Serialize)]
#[serde(transparent)]
struct PidText(#[serde(with = "super::decimal")] u64);

impl TryFrom<PidText> for Pid {
    type Error = std::num::TryFromIntError;

    fn try_from(text: PidText) -> Result<Self, Self::Error> {
        u32::try_from(text.0).map(Self)
    }
}

impl From<Pid> for PidText {
    fn from(pid: Pid) -> Self {
        Self(u64::from(pid.0))
    }
}

impl Pid {
    /// Wraps a PID reported by the OS.
    #[must_use]
    pub const fn new(pid: u32) -> Self {
        Self(pid)
    }

    /// The numeric PID.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Where a start token comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StartTokenSource {
    /// Linux boot id plus `/proc/<pid>/stat` start time in clock ticks.
    LinuxProcStat,
    /// macOS `ps -o lstart`, whose resolution is one second.
    MacosPsLstart,
}

impl StartTokenSource {
    /// The source this platform uses.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Unsupported`] where no start-token probe exists.
    pub const fn current() -> Result<Self, ProbeError> {
        if cfg!(target_os = "linux") {
            Ok(Self::LinuxProcStat)
        } else if cfg!(target_os = "macos") {
            Ok(Self::MacosPsLstart)
        } else {
            Err(ProbeError::Unsupported {
                capability: Capability::ProcessStartToken,
            })
        }
    }
}

/// Scheduler state of an observed process.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProcessState {
    /// The process can still run.
    Live,
    /// The process exited and waits to be reaped; its PID is not a live program.
    Zombie,
}

/// One observation of a process.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ProcessObservation {
    /// The observed process.
    pub pid: Pid,
    /// Token that distinguishes this process from a later one with the same PID.
    pub start_token: ProcessStartToken,
    /// Whether the process is live.
    pub state: ProcessState,
}

/// Observes `pid` through the OS process table.
///
/// # Errors
///
/// [`ProbeError::ProcessRecordUnreadable`] when the process is gone,
/// [`ProbeError::ProcessRecordMalformed`] when its record cannot be parsed and
/// [`ProbeError::Unsupported`] on platforms without a start-token probe.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos")),
    expect(
        clippy::missing_const_for_fn,
        reason = "only the unsupported-platform stub is const; Linux and macOS read the OS at run time"
    )
)]
pub fn observe_process(pid: Pid) -> Result<ProcessObservation, ProbeError> {
    os::observe(pid)
}

/// Resident memory of `pid`.
///
/// # Errors
///
/// As [`observe_process`], with [`Capability::ResidentMemory`] when unsupported.
pub fn resident_memory(pid: Pid) -> Result<ByteCount, ProbeError> {
    os::resident_kib(pid).map(|kib| ByteCount::new(kib.saturating_mul(1024)))
}

/// Ends `pid` with SIGKILL, which a program can neither catch nor ignore.
///
/// # Errors
///
/// [`ProbeError::Io`] when `kill` cannot run or reports failure.
#[cfg(unix)]
pub fn force_kill(pid: Pid) -> Result<(), ProbeError> {
    let kill_error = |source| ProbeError::Io {
        operation: super::error::PtyOperation::Kill,
        source,
    };
    let output = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(kill_error)?;
    if output.status.success() {
        Ok(())
    } else {
        let reason = String::from_utf8_lossy(&output.stderr);
        Err(kill_error(std::io::Error::other(format!(
            "kill -KILL {pid}: {}",
            reason.trim()
        ))))
    }
}

#[cfg(target_os = "linux")]
#[path = "process_linux.rs"]
mod os;

#[cfg(target_os = "macos")]
#[path = "process_macos.rs"]
mod os;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod os {
    use super::{Capability, Pid, ProbeError, ProcessObservation};

    pub(super) const fn observe(_pid: Pid) -> Result<ProcessObservation, ProbeError> {
        Err(ProbeError::Unsupported {
            capability: Capability::ProcessStartToken,
        })
    }

    pub(super) const fn resident_kib(_pid: Pid) -> Result<u64, ProbeError> {
        Err(ProbeError::Unsupported {
            capability: Capability::ResidentMemory,
        })
    }
}
