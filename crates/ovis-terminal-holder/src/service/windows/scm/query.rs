//! Read-only queries of one open service: its description and its status.

use std::io;

use ovis_terminal_protocol::domain::windows_service::{Installation, ScmState};
use ovis_terminal_protocol::schema::ids::ProcessPid;
use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows_sys::Win32::System::Services::{
    QueryServiceConfig2W, QueryServiceStatusEx, SC_STATUS_PROCESS_INFO, SERVICE_CONFIG_DESCRIPTION,
    SERVICE_DESCRIPTIONW, SERVICE_STATUS_PROCESS,
};

use super::super::HostError;
use super::super::pipe::read_wide;
use super::{Handle, last_code, oversized};

pub(super) fn description(service: &Handle) -> Result<String, HostError> {
    let mut needed = 0_u32;
    // SAFETY: a null buffer of length zero only asks for the required size.
    let sized = unsafe {
        QueryServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DESCRIPTION,
            std::ptr::null_mut(),
            0,
            &raw mut needed,
        )
    };
    if sized == 0 && last_code().1 != Some(ERROR_INSUFFICIENT_BUFFER) {
        return Err(HostError::os("read the service description", last_code().0));
    }
    let words = usize::try_from(needed).map_err(oversized)?.div_ceil(8);
    let mut buffer = vec![0_u64; words.max(1)];
    // SAFETY: the buffer is live, 8-byte aligned and at least `needed` bytes long.
    let read = unsafe {
        QueryServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DESCRIPTION,
            buffer.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    };
    if read == 0 {
        return Err(HostError::os("read the service description", last_code().0));
    }
    // SAFETY: the call filled the buffer with a SERVICE_DESCRIPTIONW.
    let text = unsafe { (*buffer.as_ptr().cast::<SERVICE_DESCRIPTIONW>()).lpDescription };
    if text.is_null() {
        return Ok(String::new());
    }
    // SAFETY: the description is NUL-terminated inside the live buffer.
    Ok(unsafe { read_wide(text) })
}

pub(super) fn status(service: &Handle) -> Result<(Installation, u32), HostError> {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let size = u32::try_from(size_of::<SERVICE_STATUS_PROCESS>()).map_err(oversized)?;
    let mut needed = 0_u32;
    // SAFETY: the service handle is open and the buffer is a live SERVICE_STATUS_PROCESS.
    let read = unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            (&raw mut status).cast(),
            size,
            &raw mut needed,
        )
    };
    if read == 0 {
        return Err(HostError::os("query the service status", last_code().0));
    }
    let state = ScmState::from_raw(status.dwCurrentState).ok_or_else(|| HostError::Io {
        operation: "query the service status",
        source: io::Error::new(io::ErrorKind::InvalidData, "undocumented service state"),
    })?;
    let pid = (status.dwProcessId != 0).then(|| ProcessPid::new(status.dwProcessId));
    Ok((Installation::new(state, pid), status.dwWin32ExitCode))
}
