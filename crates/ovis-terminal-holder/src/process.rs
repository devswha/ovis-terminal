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
pub use os::{list_terminal_pids, read_argv, read_cwd, read_entry};
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use other as os;
#[cfg(unix)]
pub use signal::{SignalError, SignalScope, send_hangup, send_signal};
pub use start_token::{StartTokenError, read_start_token};
pub use table::{Membership, ProbeError, ProcessEntry, TerminalMembership};
#[cfg(windows)]
use windows as os;

pub use crate::protocol::ProcessId;

/// `windows-created:<FILETIME>`: the creation time in 100 ns units since 1601, read
/// through an open process handle. It identifies the process even after it exited, so
/// the ConPTY holder and [`read_start_token`] produce the same token.
#[cfg(windows)]
pub(crate) fn creation_token(
    handle: windows_sys::Win32::Foundation::HANDLE,
) -> std::io::Result<crate::protocol::ProcessStartToken> {
    let mut times = [windows_sys::Win32::Foundation::FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: the caller keeps `handle` open for the call, and each pointer names a
    // distinct live local FILETIME the call writes once.
    let succeeded = unsafe {
        windows_sys::Win32::System::Threading::GetProcessTimes(
            handle, created, exited, kernel, user,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    Ok(crate::protocol::ProcessStartToken::new(format!(
        "windows-created:{ticks}"
    )))
}
