//! The service control manager's entry point for a holder service.
//!
//! Before anything else starts, the service process places itself in a job that ends
//! every process in it when the service ends. A program a ConPTY terminal starts is then
//! born inside that job, and so is any grandchild it creates before the terminal's own
//! job claims it, so nothing can escape the holder between `CreateProcessW` and
//! `AssignProcessToJobObject`. The manager, not any client or SSH session, started this
//! process, so none of their jobs contain it.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{ERROR_SERVICE_SPECIFIC_ERROR, NO_ERROR};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Services::{
    RegisterServiceCtrlHandlerW, SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
    SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS, StartServiceCtrlDispatcherW,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use super::host::Stopper;
use super::pipe::wide;
use super::{HolderHost, HostConfig, HostError};

static CONFIG: OnceLock<HostConfig> = OnceLock::new();
static STOPPER: OnceLock<Stopper> = OnceLock::new();
static OUTCOME: Mutex<Option<Result<(), HostError>>> = Mutex::new(None);
/// Held until the process exits; closing it ends every process of the holder.
static HOST_JOB: OnceLock<OwnedHandle> = OnceLock::new();

fn io(operation: &'static str) -> HostError {
    HostError::os(operation, std::io::Error::last_os_error())
}

fn enter_host_job() -> Result<(), HostError> {
    // SAFETY: null attributes and name create an unnamed job with default security.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(io("create the holder job"));
    }
    // SAFETY: the call returned a new handle that nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let length = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        .map_err(|_| io("size the job limits"))?;
    // SAFETY: the job is open and the pointer and length describe the live `limits`.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            length,
        )
    } == 0
    {
        return Err(io("limit the holder job"));
    }
    // SAFETY: GetCurrentProcess has no preconditions; its pseudo-handle needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: both handles are open for the call.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
        return Err(io("enter the holder job"));
    }
    HOST_JOB
        .set(job)
        .map_err(|_| io("enter the holder job twice"))
}

struct Reporter(SERVICE_STATUS_HANDLE);

impl Reporter {
    fn report(&self, state: u32, exit_code: u32) {
        let accepted = if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        };
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: accepted,
            dwWin32ExitCode: exit_code,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: if state == SERVICE_RUNNING || state == SERVICE_STOPPED {
                0
            } else {
                30_000
            },
        };
        // SAFETY: the status handle came from RegisterServiceCtrlHandlerW and `status`
        // is a live local. A failed report leaves the manager's view stale but changes
        // nothing the holder owns.
        unsafe {
            windows_sys::Win32::System::Services::SetServiceStatus(self.0, &raw const status)
        };
    }
}

unsafe extern "system" fn control(code: u32) {
    if matches!(code, SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN)
        && let Some(stopper) = STOPPER.get()
    {
        stopper.stop();
    }
}

fn host(reporter: &Reporter, config: &HostConfig) -> Result<(), HostError> {
    reporter.report(SERVICE_START_PENDING, NO_ERROR);
    enter_host_job()?;
    let holder = HolderHost::start(config.clone())?;
    STOPPER
        .set(holder.stopper())
        .map_err(|_| io("start the holder twice"))?;
    reporter.report(SERVICE_RUNNING, NO_ERROR);
    holder.wait();
    reporter.report(SERVICE_STOP_PENDING, NO_ERROR);
    holder.stop()
}

unsafe extern "system" fn service_main(_: u32, _: *mut windows_sys::core::PWSTR) {
    let Some(config) = CONFIG.get() else {
        return;
    };
    let name = wide(config.paths().session().label().as_str());
    // SAFETY: `name` is NUL-terminated and `control` has the handler signature.
    let handle = unsafe { RegisterServiceCtrlHandlerW(name.as_ptr(), Some(control)) };
    if handle.is_null() {
        *OUTCOME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Err(io("register the service handler")));
        return;
    }
    let reporter = Reporter(handle);
    let outcome = host(&reporter, config);
    reporter.report(
        SERVICE_STOPPED,
        if outcome.is_ok() {
            NO_ERROR
        } else {
            ERROR_SERVICE_SPECIFIC_ERROR
        },
    );
    *OUTCOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
}

/// Runs the holder of `config` as the service the manager started; returns when it stops.
///
/// # Errors
///
/// A process the manager did not start cannot connect to it; that and the holder's own
/// failure are returned.
pub fn run_service(config: HostConfig) -> Result<(), HostError> {
    let mut name = wide(config.paths().session().label().as_str());
    CONFIG
        .set(config)
        .map_err(|_| io("run the service twice"))?;
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: std::ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    // SAFETY: the table is null-terminated and its name outlives the dispatcher, which
    // returns only after the service stopped.
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        return Err(io("connect to the service control manager"));
    }
    OUTCOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .unwrap_or(Ok(()))
}
