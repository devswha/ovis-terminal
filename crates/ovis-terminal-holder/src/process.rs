//! Process incarnation evidence gathered by the holder (plan todos 6 and 55).
//!
//! A holder reports each child by PID plus an operating-system start token, so
//! a reused PID is never mistaken for the process the holder started.

#[cfg(target_os = "macos")]
mod darwin;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod other;
#[cfg(unix)]
mod signal;
mod start_token;
mod table;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use darwin as os;
#[cfg(target_os = "linux")]
use linux as os;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use other as os;
#[cfg(unix)]
pub use signal::{SignalError, SignalScope, send_hangup, send_signal};
pub use start_token::{StartTokenError, read_start_token};
pub use table::{
    Membership, ProbeError, ProcessEntry, TerminalMembership, list_terminal_pids, read_argv,
    read_cwd, read_entry,
};
#[cfg(windows)]
use windows as os;
#[cfg(windows)]
pub(crate) use windows::creation_token;

pub use crate::protocol::ProcessId;
