//! Explicit registrations with systemd user services and launchd agents.
//!
//! Only `install`, `uninstall`, `start` and `stop` change a manager; reading the
//! registration and its prerequisites calls read-only manager queries at most.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ovis_terminal_protocol::domain::unix_service::Prerequisite;
use ovis_terminal_protocol::schema::methods::service::ServiceManager;

use super::manager::{call, checked};
use super::{ServiceError, ServicePaths, definition, record};

const QUERY_BOUND: Duration = Duration::from_secs(5);
const START_BOUND: Duration = Duration::from_secs(20);

fn supported(manager: ServiceManager) -> Result<(), ServiceError> {
    let native = match manager {
        ServiceManager::SystemdUser => cfg!(target_os = "linux"),
        ServiceManager::Launchd => cfg!(target_os = "macos"),
        _ => false,
    };
    native.then_some(()).ok_or(ServiceError::Unsupported)
}

fn uid() -> String {
    rustix::process::getuid().as_raw().to_string()
}

fn answers(program: &'static str, args: &[&str]) -> bool {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    call(program, &args, QUERY_BOUND).is_ok_and(|called| called.success)
}

/// Unmet conditions for a holder under `manager` to outlive a logout; `None` (no manager)
/// always lacks one, since a detached process is not logout-safe.
pub fn prerequisites(manager: Option<ServiceManager>) -> Vec<Prerequisite> {
    let Some(manager) = manager.filter(|manager| supported(*manager).is_ok()) else {
        return vec![Prerequisite::ServiceManager];
    };
    let mut missing = Vec::new();
    if manager == ServiceManager::SystemdUser {
        if !answers("systemctl", &["--user", "show-environment"]) {
            missing.push(Prerequisite::SystemdUserManager);
        }
        let linger = call(
            "loginctl",
            &[
                OsStr::new("show-user"),
                OsStr::new(&uid()),
                OsStr::new("--property=Linger"),
                OsStr::new("--value"),
            ],
            QUERY_BOUND,
        );
        if !linger.is_ok_and(|called| called.success && called.stdout.trim() == "yes") {
            missing.push(Prerequisite::UserLinger);
        }
    } else if !answers("launchctl", &["print", &format!("gui/{}", uid())]) {
        missing.push(Prerequisite::LaunchdUserDomain);
    }
    missing
}

fn read_owned(path: &Path, paths: &ServicePaths) -> Result<Option<bool>, ServiceError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(definition::owned(&text, paths))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ServiceError::Io {
            operation: "read the service definition",
            source,
        }),
    }
}

/// The manager holding a registration this runtime wrote for `paths`, if any.
///
/// # Errors
///
/// A definition file that cannot be read.
pub fn registered(paths: &ServicePaths) -> Result<Option<ServiceManager>, ServiceError> {
    for manager in [ServiceManager::SystemdUser, ServiceManager::Launchd] {
        let Ok(path) = definition::location(paths, manager) else {
            continue;
        };
        if read_owned(&path, paths)? == Some(true) {
            return Ok(Some(manager));
        }
    }
    Ok(None)
}

fn require_manager(manager: ServiceManager) -> Result<(), ServiceError> {
    supported(manager)?;
    let missing = prerequisites(Some(manager));
    match missing.first() {
        Some(first @ (Prerequisite::SystemdUserManager | Prerequisite::LaunchdUserDomain)) => {
            Err(ServiceError::ManagerUnavailable { missing: *first })
        }
        _ => Ok(()),
    }
}

/// Writes the definition for `manager` and makes the manager read it; nothing starts.
///
/// # Errors
///
/// [`ServiceError::Unsupported`], [`ServiceError::ManagerUnavailable`] (no file is
/// written), [`ServiceError::ForeignRegistration`], or a write or manager failure (the
/// written file is removed again).
pub fn install(
    paths: &ServicePaths,
    manager: ServiceManager,
    program: &Path,
) -> Result<PathBuf, ServiceError> {
    require_manager(manager)?;
    let path = definition::location(paths, manager)?;
    if read_owned(&path, paths)? == Some(false) {
        return Err(ServiceError::ForeignRegistration { path });
    }
    let text = match manager {
        ServiceManager::SystemdUser => definition::systemd(paths, program)?,
        _ => definition::launchd(paths, program)?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ServiceError::Io {
            operation: "create the definition directory",
            source,
        })?;
    }
    std::fs::write(&path, text).map_err(|source| ServiceError::Io {
        operation: "write the service definition",
        source,
    })?;
    if manager == ServiceManager::SystemdUser
        && let Err(error) = checked(
            "systemctl",
            &[OsStr::new("--user"), OsStr::new("daemon-reload")],
            QUERY_BOUND,
        )
    {
        record::remove(&path)?;
        return Err(error);
    }
    Ok(path)
}

/// Removes this runtime's definition for `paths`; a running holder must be stopped first.
///
/// # Errors
///
/// [`ServiceError::NotRegistered`], [`ServiceError::ForeignRegistration`],
/// [`ServiceError::AlreadyRunning`], or a manager or filesystem failure.
pub fn uninstall(paths: &ServicePaths, manager: ServiceManager) -> Result<PathBuf, ServiceError> {
    supported(manager)?;
    let path = definition::location(paths, manager)?;
    match read_owned(&path, paths)? {
        None => return Err(ServiceError::NotRegistered),
        Some(false) => return Err(ServiceError::ForeignRegistration { path }),
        Some(true) => {}
    }
    super::run::refuse_live(paths)?;
    if manager == ServiceManager::Launchd {
        let target = format!("gui/{}/{}", uid(), paths.session().launchd_label());
        call(
            "launchctl",
            &[OsStr::new("bootout"), OsStr::new(&target)],
            QUERY_BOUND,
        )?;
    }
    record::remove(&path)?;
    if manager == ServiceManager::SystemdUser {
        checked(
            "systemctl",
            &[OsStr::new("--user"), OsStr::new("daemon-reload")],
            QUERY_BOUND,
        )?;
    }
    Ok(path)
}

/// Asks `manager` to start the registered holder; returns once the manager reports it.
///
/// # Errors
///
/// [`ServiceError::NotRegistered`] or the manager failure.
pub fn start(paths: &ServicePaths, manager: ServiceManager) -> Result<(), ServiceError> {
    if registered(paths)? != Some(manager) {
        return Err(ServiceError::NotRegistered);
    }
    require_manager(manager)?;
    let path = definition::location(paths, manager)?;
    if manager == ServiceManager::SystemdUser {
        let unit = paths.systemd_unit_name();
        let args = [OsStr::new("--user"), OsStr::new("start"), OsStr::new(&unit)];
        checked("systemctl", &args, START_BOUND)?;
    } else {
        let domain = format!("gui/{}", uid());
        let args = [
            OsStr::new("bootstrap"),
            OsStr::new(&domain),
            path.as_os_str(),
        ];
        checked("launchctl", &args, START_BOUND)?;
    }
    Ok(())
}

/// Returns the manager's record of the unit to inactive after the holder itself stopped.
///
/// # Errors
///
/// The manager failure.
pub fn settle(paths: &ServicePaths, manager: ServiceManager) -> Result<(), ServiceError> {
    if manager == ServiceManager::SystemdUser {
        let unit = paths.systemd_unit_name();
        let args = [OsStr::new("--user"), OsStr::new("stop"), OsStr::new(&unit)];
        checked("systemctl", &args, QUERY_BOUND).map(drop)
    } else {
        let target = format!("gui/{}/{}", uid(), paths.session().launchd_label());
        call(
            "launchctl",
            &[OsStr::new("bootout"), OsStr::new(&target)],
            QUERY_BOUND,
        )
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::domain::unix_service::Prerequisite;
    use ovis_terminal_protocol::schema::methods::service::ServiceManager;

    use super::{install, prerequisites, registered, uninstall};
    use crate::service::unix::test_root::TestRoot;

    #[test]
    fn a_detached_holder_always_lacks_a_manager_and_the_other_os_manager_is_unsupported() {
        assert_eq!(prerequisites(None), vec![Prerequisite::ServiceManager]);
        let root = TestRoot::new("foreign-os");
        let paths = root.paths("s");
        let other = if cfg!(target_os = "macos") {
            ServiceManager::SystemdUser
        } else {
            ServiceManager::Launchd
        };
        let refused = install(&paths, other, std::path::Path::new("/bin/true"))
            .map(drop)
            .map_err(|e| e.kind());
        assert_eq!(refused, Err("service_unsupported"));
        assert_eq!(
            prerequisites(Some(other)),
            vec![Prerequisite::ServiceManager]
        );
    }

    #[test]
    fn a_definition_this_runtime_did_not_write_is_never_reported_or_removed() {
        let root = TestRoot::new("foreign");
        let paths = root.paths("s");
        let native = if cfg!(target_os = "macos") {
            ServiceManager::Launchd
        } else {
            ServiceManager::SystemdUser
        };
        let path = crate::service::unix::definition::location(&paths, native).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[Unit]\nDescription=not ours\n").unwrap();
        assert_eq!(registered(&paths).unwrap(), None);
        assert_eq!(
            uninstall(&paths, native).map(drop).map_err(|e| e.kind()),
            Err("foreign_registration")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Unit]\nDescription=not ours\n"
        );
    }
}
