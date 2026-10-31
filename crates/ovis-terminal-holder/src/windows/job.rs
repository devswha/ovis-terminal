//! A job object that owns one terminal's process tree.
//!
//! The job is created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so the tree cannot
//! outlive the holder that owns it, and kill terminates the whole tree rather than only
//! the process ConPTY started.

use std::io;
use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

/// An owned job handle; dropping it closes the handle and ends every process in it.
#[derive(Debug)]
pub(super) struct Job {
    handle: OwnedHandle,
}

fn size_of_u32<T>() -> io::Result<u32> {
    u32::try_from(size_of::<T>()).map_err(io::Error::other)
}

fn check(succeeded: i32) -> io::Result<()> {
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Job {
    /// Creates an unnamed job that kills its processes when its last handle closes.
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: both arguments may be null: default security and no name.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a valid handle CreateJobObjectW just returned to this caller,
        // and nothing else owns or closes it.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let job = Self { handle };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let length = size_of_u32::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()?;
        // SAFETY: the job handle is open, and the pointer and length describe the live
        // local `limits` of exactly the type the information class names.
        check(unsafe {
            SetInformationJobObject(
                job.handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                length,
            )
        })?;
        Ok(job)
    }

    /// Places `process` and every child it creates afterwards in this job.
    pub(super) fn assign(&self, process: BorrowedHandle<'_>) -> io::Result<()> {
        // SAFETY: both handles are open for the duration of the call: the job by
        // `self`, the process by the borrow.
        check(unsafe {
            AssignProcessToJobObject(self.handle.as_raw_handle(), process.as_raw_handle())
        })
    }

    /// Terminates every process in the job with `code`.
    pub(super) fn terminate(&self, code: u32) -> io::Result<()> {
        // SAFETY: the job handle is open for the duration of the call.
        check(unsafe { TerminateJobObject(self.handle.as_raw_handle(), code) })
    }

    /// Processes currently in the job.
    #[cfg(test)]
    pub(super) fn active_processes(&self) -> io::Result<u32> {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };

        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let length = size_of_u32::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()?;
        // SAFETY: the job handle is open, the pointer and length describe the live local
        // `accounting` of the type the information class names, and a null return-length
        // pointer is allowed.
        check(unsafe {
            QueryInformationJobObject(
                self.handle.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                length,
                std::ptr::null_mut(),
            )
        })?;
        Ok(accounting.ActiveProcesses)
    }
}
