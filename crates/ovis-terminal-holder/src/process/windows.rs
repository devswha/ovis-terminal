//! Windows process records: the process object's creation time and exit state.
//!
//! Windows has no process groups or controlling terminals, so terminal membership,
//! process-table enumeration, argv and cwd are reported as unsupported.

use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::table::{Membership, ProbeError, ProcessEntry};
use super::{ProcessId, StartTokenError};
use crate::protocol::ProcessStartToken;

/// `windows-created:<FILETIME>`: the creation time in 100 ns units since 1601, read
/// through an open process handle.
fn creation_token(handle: HANDLE) -> io::Result<ProcessStartToken> {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: the caller keeps `handle` open for the call, and each pointer names a
    // distinct live local FILETIME the call writes once.
    let succeeded = unsafe { GetProcessTimes(handle, created, exited, kernel, user) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    Ok(ProcessStartToken::new(format!("windows-created:{ticks}")))
}

fn os_error(pid: ProcessId, source: io::Error) -> ProbeError {
    let code = source
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok());
    match code {
        Some(ERROR_INVALID_PARAMETER) => ProbeError::Gone { pid: pid.get() },
        Some(ERROR_ACCESS_DENIED) => ProbeError::Permission { source },
        _ => ProbeError::Unreadable { source },
    }
}

fn open(pid: ProcessId) -> Result<OwnedHandle, ProbeError> {
    // SAFETY: OpenProcess takes plain values and returns a new handle or null.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid.get()) };
    if raw.is_null() {
        return Err(os_error(pid, io::Error::last_os_error()));
    }
    // SAFETY: `raw` is a freshly opened handle owned by nobody else.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

fn running(pid: ProcessId, handle: &OwnedHandle) -> Result<bool, ProbeError> {
    let mut code = 0_u32;
    // SAFETY: the handle is open and `code` is a live local the call writes once.
    let succeeded = unsafe { GetExitCodeProcess(handle.as_raw_handle(), &raw mut code) };
    if succeeded == 0 {
        return Err(os_error(pid, io::Error::last_os_error()));
    }
    Ok(i32::try_from(code).is_ok_and(|code| code == STILL_ACTIVE))
}

pub(super) fn start_token(pid: ProcessId) -> Result<ProcessStartToken, StartTokenError> {
    let handle = open(pid)?;
    creation_token(handle.as_raw_handle()).map_err(|source| StartTokenError::Unreadable { source })
}

/// Reads the record of `pid`.
///
/// # Errors
///
/// Returns [`ProbeError::Gone`] when no process has the PID, or a typed read failure.
pub fn read_entry(pid: ProcessId) -> Result<ProcessEntry, ProbeError> {
    let handle = open(pid)?;
    let start_token =
        creation_token(handle.as_raw_handle()).map_err(|source| os_error(pid, source))?;
    Ok(ProcessEntry {
        pid,
        start_token,
        running: running(pid, &handle)?,
        terminal: Membership::Unsupported,
    })
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
