//! The read-only operating-system facts an observation needs, and observing a service.

use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use ovis_terminal_protocol::domain::unix_service::{BootProof, Observed};
use ovis_terminal_protocol::domain::windows_service::{HolderPlacement, PipeName};
use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken};
use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, FILETIME, STILL_ACTIVE};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{Facts, HostError, HostPaths, PipePresence, ProcessCheck, judge, pipe, read_record};

/// Facts read from this host's process table and pipe namespace.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct OsFacts;

fn creation_token(process: &OwnedHandle) -> io::Result<String> {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: the process handle is open and each pointer names a distinct live local.
    let succeeded =
        unsafe { GetProcessTimes(process.as_raw_handle(), created, exited, kernel, user) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    Ok(format!("windows-created:{ticks}"))
}

fn check_process(pid: u32) -> io::Result<ProcessCheck> {
    // SAFETY: OpenProcess has no memory preconditions; a null result is handled below.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        let error = io::Error::last_os_error();
        let gone = error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
            == Some(ERROR_INVALID_PARAMETER);
        return if gone {
            Ok(ProcessCheck::Gone)
        } else {
            Err(error)
        };
    }
    // SAFETY: OpenProcess returned a new handle that nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut code = 0_u32;
    // SAFETY: the handle is open and `code` is a live local.
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // A process object outlives its exit while any handle to it is open.
    if code != u32::try_from(STILL_ACTIVE).map_err(io::Error::other)? {
        return Ok(ProcessCheck::Gone);
    }
    Ok(ProcessCheck::Alive {
        start_token: creation_token(&process)?,
    })
}

impl Facts for OsFacts {
    fn process(&self, pid: ProcessPid) -> Result<ProcessCheck, HostError> {
        check_process(pid.get())
            .map_err(|source| HostError::os("inspect the holder process", source))
    }

    fn pipe(&self, pipe: &PipeName) -> Result<PipePresence, HostError> {
        pipe::presence(pipe).map_err(|source| HostError::os("probe the holder pipe", source))
    }
}

/// This process's boot proof: its PID and creation-time start token.
///
/// # Errors
///
/// The process query failure.
pub(super) fn own_proof() -> Result<BootProof, HostError> {
    let pid = std::process::id();
    match check_process(pid) {
        Ok(ProcessCheck::Alive { start_token }) => Ok(BootProof::new(
            ProcessPid::new(pid),
            StartToken::new(start_token),
        )),
        Ok(ProcessCheck::Gone) => Err(HostError::Io {
            operation: "read this process's start time",
            source: io::ErrorKind::NotFound.into(),
        }),
        Err(source) => Err(HostError::os("read this process's start time", source)),
    }
}

/// Observes the holder of `paths` owned by `owner_sid`; nothing is created or started.
///
/// # Errors
///
/// A record that cannot be parsed, or a process or pipe query that failed.
pub fn observe(
    paths: &HostPaths,
    owner_sid: &str,
) -> Result<(Observed, Option<HolderPlacement>), HostError> {
    judge(
        read_record(&paths.record())?,
        &paths.pipe(owner_sid)?,
        &OsFacts,
    )
}
