//! The holder's own handle to a program ConPTY started, and its start evidence.
//!
//! Holding a process handle keeps the PID from being reused, so the PID plus the
//! process creation time read through that handle identify exactly the program the
//! holder started.

use std::io;
use std::os::windows::io::{AsHandle as _, AsRawHandle as _, BorrowedHandle, OwnedHandle};

use portable_pty::Child;
use windows_sys::Win32::System::Threading::GetProcessId;

use crate::protocol::ProcessStartToken;

/// An owned duplicate of a started program's process handle.
#[derive(Debug)]
pub(super) struct ProcessHandle {
    handle: OwnedHandle,
}

impl ProcessHandle {
    /// Duplicates the process handle `child` holds.
    pub(super) fn of(child: &dyn Child) -> io::Result<Self> {
        let raw = child.as_raw_handle().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "the child exposes no process handle",
            )
        })?;
        // SAFETY: `raw` is the open process handle owned by `child`, which is borrowed
        // for longer than this borrow; the duplicate below is independently owned.
        let borrowed = unsafe { BorrowedHandle::borrow_raw(raw) };
        Ok(Self {
            handle: borrowed.try_clone_to_owned()?,
        })
    }

    pub(super) fn borrow(&self) -> BorrowedHandle<'_> {
        self.handle.as_handle()
    }

    /// The process identifier.
    pub(super) fn pid(&self) -> io::Result<u32> {
        // SAFETY: the process handle is open for the duration of the call.
        let pid = unsafe { GetProcessId(self.handle.as_raw_handle()) };
        if pid == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    /// `windows-created:<FILETIME>`, in the form `process::read_start_token` reads.
    pub(super) fn start_token(&self) -> io::Result<ProcessStartToken> {
        crate::process::creation_token(self.handle.as_raw_handle())
    }
}

/// Handles the current process holds open.
#[cfg(test)]
pub(super) fn open_handle_count() -> io::Result<u32> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

    let mut count = 0;
    // SAFETY: GetCurrentProcess has no preconditions; its pseudo-handle needs no closing.
    let current = unsafe { GetCurrentProcess() };
    // SAFETY: `current` names this process and the pointer names the live local `count`.
    let succeeded = unsafe { GetProcessHandleCount(current, &raw mut count) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(count)
}
