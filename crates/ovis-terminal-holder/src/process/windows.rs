//! Windows process records: the process object's creation time and exit state.
//!
//! A process object outlives its process while any handle to it is open, so exit is
//! read from the object's exit code; an absent PID fails `OpenProcess` with
//! `ERROR_INVALID_PARAMETER`.
//!
//! Windows has no process groups or controlling terminals, so terminal membership,
//! process-table enumeration, argv and cwd are reported as unsupported.

use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, STILL_ACTIVE};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::table::{Membership, ProbeError, ProcessEntry};
use super::{ProcessId, creation_token};

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
