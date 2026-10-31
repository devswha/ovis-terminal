//! Explicit, scoped administration through the service control manager.
//!
//! Every verb opens the service by its label and first checks the description marker,
//! so it never stops, starts or deletes a same-named service this runtime did not
//! install for this owner. Waits for a state are bounded; an expired wait reports that
//! the outcome is unknown instead of guessing.

use std::io;
use std::time::{Duration, Instant};

use ovis_terminal_protocol::domain::windows_service::{Installation, ScmState, ServiceAccount};
use windows_sys::Win32::Foundation::{
    ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_EXISTS,
    ERROR_SERVICE_NOT_ACTIVE,
};
use windows_sys::Win32::Storage::FileSystem::DELETE;
use windows_sys::Win32::System::Services::{
    ChangeServiceConfig2W, CloseServiceHandle, ControlService, CreateServiceW, DeleteService,
    OpenSCManagerW, OpenServiceW, SC_HANDLE, SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE,
    SERVICE_CHANGE_CONFIG, SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTROL_STOP, SERVICE_DEMAND_START,
    SERVICE_DESCRIPTIONW, SERVICE_ERROR_NORMAL, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS,
    SERVICE_START, SERVICE_STATUS, SERVICE_STOP, SERVICE_WIN32_OWN_PROCESS, StartServiceW,
};

use super::pipe::wide;
use super::{HostError, HostPaths, ServiceDefinition, marker};

/// A size that does not fit the system call's integer type.
pub(super) fn oversized(error: impl std::error::Error + Send + Sync + 'static) -> HostError {
    HostError::os("size a system buffer", io::Error::other(error))
}

mod query;
mod rights;

use query::{description, status};

/// The SID of `account`, as text: the owner its service and pipe are installed for.
///
/// # Errors
///
/// The account lookup failure.
pub fn account_sid(account: &ServiceAccount) -> Result<String, HostError> {
    rights::account_sid_string(account)
}

const POLL: Duration = Duration::from_millis(25);

struct Handle(SC_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned open by the manager and is closed once.
        unsafe { CloseServiceHandle(self.0) };
    }
}

fn last_code() -> (io::Error, Option<u32>) {
    let error = io::Error::last_os_error();
    let code = error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok());
    (error, code)
}

fn manager(access: u32) -> Result<Handle, HostError> {
    // SAFETY: null machine and database names select the local active database.
    let raw = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), access) };
    if raw.is_null() {
        return Err(HostError::os(
            "open the service control manager",
            last_code().0,
        ));
    }
    Ok(Handle(raw))
}

fn open(manager: &Handle, name: &str, access: u32) -> Result<Option<Handle>, HostError> {
    let name = wide(name);
    // SAFETY: the manager handle is open and `name` is NUL-terminated.
    let raw = unsafe { OpenServiceW(manager.0, name.as_ptr(), access) };
    if !raw.is_null() {
        return Ok(Some(Handle(raw)));
    }
    match last_code() {
        (_, Some(ERROR_SERVICE_DOES_NOT_EXIST)) => Ok(None),
        (error, _) => Err(HostError::os("open the service", error)),
    }
}

/// Opens this owner's service of `paths` with `access`, refusing a foreign one.
fn owned(paths: &HostPaths, owner_sid: &str, access: u32) -> Result<Handle, HostError> {
    let label = paths.session().label();
    let service = open(
        &manager(SC_MANAGER_CONNECT)?,
        label.as_str(),
        access | SERVICE_QUERY_CONFIG,
    )?
    .ok_or(HostError::NotInstalled)?;
    if description(&service)? != marker(label.as_str(), owner_sid) {
        return Err(HostError::ForeignRegistration {
            label: label.as_str().to_owned(),
        });
    }
    Ok(service)
}

fn wait(service: &Handle, wanted: ScmState, bound: Duration) -> Result<Installation, HostError> {
    let deadline = Instant::now().checked_add(bound);
    loop {
        let (installation, exit_code) = status(service)?;
        if installation.state == wanted {
            return Ok(installation);
        }
        if installation.state == ScmState::Stopped {
            return Err(HostError::ServiceFailed { code: exit_code });
        }
        if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
            return Err(HostError::StateTimeout {
                wanted,
                bound_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
            });
        }
        std::thread::sleep(POLL);
    }
}

/// The manager's view of this owner's service of `paths`; `None` when none is installed.
///
/// # Errors
///
/// [`HostError::ForeignRegistration`] for a same-named service another owner installed,
/// and query failures.
pub fn installation(paths: &HostPaths, owner_sid: &str) -> Result<Option<Installation>, HostError> {
    let label = paths.session().label();
    let access = SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG;
    let Some(service) = open(&manager(SC_MANAGER_CONNECT)?, label.as_str(), access)? else {
        return Ok(None);
    };
    if description(&service)? != marker(label.as_str(), owner_sid) {
        return Err(HostError::ForeignRegistration {
            label: label.as_str().to_owned(),
        });
    }
    Ok(Some(status(&service)?.0))
}

/// Installs `definition` to run as `account`, granting it the service logon right.
///
/// # Errors
///
/// [`HostError::AlreadyInstalled`], [`HostError::AccessDenied`] without administrator
/// rights, and the manager's failures; a half-made service is deleted again.
pub fn install(
    definition: &ServiceDefinition,
    account: &ServiceAccount,
    password: &str,
) -> Result<(), HostError> {
    rights::grant_service_logon(account)?;
    let manager = manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
    let (name, display, command) = (
        wide(&definition.name),
        wide(&definition.display_name),
        wide(&definition.command_line),
    );
    let (user, secret) = (wide(account.as_str()), wide(password));
    // SAFETY: the manager handle is open, every string is NUL-terminated and live, and the
    // optional group, tag and dependency arguments are null.
    let raw = unsafe {
        CreateServiceW(
            manager.0,
            name.as_ptr(),
            display.as_ptr(),
            SERVICE_CHANGE_CONFIG | DELETE | SERVICE_QUERY_CONFIG,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            command.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            user.as_ptr(),
            secret.as_ptr(),
        )
    };
    if raw.is_null() {
        return Err(match last_code() {
            (_, Some(ERROR_SERVICE_EXISTS)) => HostError::AlreadyInstalled {
                label: definition.name.clone(),
            },
            (error, _) => HostError::os("create the service", error),
        });
    }
    let service = Handle(raw);
    let mut text = wide(&definition.description);
    let described = SERVICE_DESCRIPTIONW {
        lpDescription: text.as_mut_ptr(),
    };
    // SAFETY: the service handle is open and `described` points at a live string.
    if unsafe {
        ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DESCRIPTION,
            (&raw const described).cast(),
        )
    } == 0
    {
        let error = last_code().0;
        // SAFETY: the handle was opened with DELETE; the rollback result is not needed
        // because the description failure is what gets reported.
        unsafe { DeleteService(service.0) };
        return Err(HostError::os("mark the service as ours", error));
    }
    Ok(())
}

/// Deletes this owner's stopped service of `paths`.
///
/// # Errors
///
/// [`HostError::NotInstalled`], [`HostError::ForeignRegistration`], and
/// [`HostError::AlreadyRunning`] while it runs.
pub fn uninstall(paths: &HostPaths, owner_sid: &str) -> Result<(), HostError> {
    let service = owned(paths, owner_sid, DELETE | SERVICE_QUERY_STATUS)?;
    if status(&service)?.0.state != ScmState::Stopped {
        return Err(HostError::AlreadyRunning { holder: None });
    }
    // SAFETY: the handle was opened with DELETE.
    if unsafe { DeleteService(service.0) } == 0 {
        return Err(HostError::os("delete the service", last_code().0));
    }
    Ok(())
}

/// Starts this owner's service of `paths` and waits, bounded, until it runs.
///
/// # Errors
///
/// [`HostError::AlreadyRunning`], [`HostError::ServiceFailed`] when it stopped instead,
/// and [`HostError::StateTimeout`] when the bound expired.
pub fn start(
    paths: &HostPaths,
    owner_sid: &str,
    bound: Duration,
) -> Result<Installation, HostError> {
    let service = owned(paths, owner_sid, SERVICE_START | SERVICE_QUERY_STATUS)?;
    // SAFETY: the handle was opened with SERVICE_START; no arguments are passed.
    if unsafe { StartServiceW(service.0, 0, std::ptr::null()) } == 0 {
        return Err(match last_code() {
            (_, Some(ERROR_SERVICE_ALREADY_RUNNING)) => HostError::AlreadyRunning { holder: None },
            (error, _) => HostError::os("start the service", error),
        });
    }
    wait(&service, ScmState::Running, bound)
}

/// Stops this owner's service of `paths` and waits, bounded, until it has stopped.
///
/// # Errors
///
/// [`HostError::NotRunning`] when it is not running and [`HostError::StateTimeout`] when
/// the bound expired.
pub fn stop(
    paths: &HostPaths,
    owner_sid: &str,
    bound: Duration,
) -> Result<Installation, HostError> {
    let service = owned(paths, owner_sid, SERVICE_STOP | SERVICE_QUERY_STATUS)?;
    let mut reported = SERVICE_STATUS::default();
    // SAFETY: the handle was opened with SERVICE_STOP and `reported` is a live local.
    if unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &raw mut reported) } == 0 {
        return Err(match last_code() {
            (_, Some(ERROR_SERVICE_NOT_ACTIVE)) => HostError::NotRunning,
            (error, _) => HostError::os("stop the service", error),
        });
    }
    wait(&service, ScmState::Stopped, bound)
}
