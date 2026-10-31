//! Running one service verb against the session's isolated service paths.

use std::io::Write as _;
use std::time::{Duration, Instant};

use ovis_terminal_holder::service::unix::{
    self as service, ServiceEnvironment, ServiceError, ServicePaths, ServiceSession,
};
use ovis_terminal_protocol::domain::unix_service::{Observed, ServiceStatus};
use ovis_terminal_protocol::schema::methods::service::ServiceManager;
use serde_json::{Value, json};

use super::args::{Manager, Placement, ServiceCommand, SessionArgs};

/// Bound on a launchd-started holder reporting running; launchd has no readiness signal.
const LAUNCHD_READY: Duration = Duration::from_secs(10);
const LAUNCHD_POLL: Duration = Duration::from_millis(20);

fn paths(args: &SessionArgs) -> Result<ServicePaths, ServiceError> {
    ServicePaths::resolve(
        ServiceEnvironment::from_process(),
        ServiceSession::parse(&args.session)?,
    )
}

fn encode(value: &impl serde::Serialize) -> Result<Value, ServiceError> {
    serde_json::to_value(value).map_err(|source| ServiceError::Io {
        operation: "encode the result",
        source: source.into(),
    })
}

fn program() -> Result<std::path::PathBuf, ServiceError> {
    std::env::current_exe().map_err(|source| ServiceError::Io {
        operation: "locate this executable",
        source,
    })
}

/// One observation with its registration facts; it changes nothing.
fn status(paths: &ServicePaths) -> Result<ServiceStatus, ServiceError> {
    let observed = service::observe(paths)?;
    let registered = service::registered(paths)?;
    let placed = match &observed {
        Observed::Running { record }
        | Observed::Unreachable {
            record: Some(record),
        } => record.manager,
        _ => registered,
    };
    let missing = service::prerequisites(placed);
    Ok(
        ServiceStatus::new(paths.session().label(), observed)
            .with_registration(registered, missing),
    )
}

fn await_launchd(paths: &ServicePaths) -> Result<(), ServiceError> {
    let deadline = Instant::now().checked_add(LAUNCHD_READY);
    loop {
        if matches!(service::observe(paths)?, Observed::Running { .. }) {
            return Ok(());
        }
        if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
            return Err(ServiceError::NotReady {
                bound_ms: u64::try_from(LAUNCHD_READY.as_millis()).unwrap_or(u64::MAX),
            });
        }
        std::thread::sleep(LAUNCHD_POLL);
    }
}

fn start(paths: &ServicePaths, placement: Placement) -> Result<Value, ServiceError> {
    service::refuse_live(paths)?;
    match placement.manager() {
        None => service::launch_detached(paths, &program()?).map(drop)?,
        Some(manager) => {
            service::start(paths, manager.protocol())?;
            if manager == Manager::Launchd {
                await_launchd(paths)?;
            }
        }
    }
    let started = status(paths)?;
    match &started.observed {
        Observed::Running { .. } => encode(&started),
        Observed::StaleEndpoint { reason, .. } => Err(ServiceError::Stale { reason: *reason }),
        Observed::Stopped => Err(ServiceError::NotRunning),
        _ => Err(ServiceError::Unreachable),
    }
}

fn stop(paths: &ServicePaths) -> Result<Value, ServiceError> {
    let manager = match service::observe(paths)? {
        Observed::Running { record } => record.manager,
        _ => None,
    };
    let receipt = service::stop(paths)?;
    if let Some(manager) = manager {
        service::settle(paths, manager)?;
    }
    encode(&receipt)
}

/// The holder process: serves until a `service stop`, announcing readiness its own way.
fn run(paths: &ServicePaths, placement: Placement) -> Result<Value, ServiceError> {
    let manager = placement.manager().map(Manager::protocol);
    service::run(paths, manager, |record| match manager {
        None => {
            let line = service::ready_line(record);
            let mut stdout = std::io::stdout().lock();
            stdout
                .write_all(line.as_bytes())
                .and_then(|()| stdout.flush())
                .map_err(|source| ServiceError::Io {
                    operation: "announce readiness",
                    source,
                })
        }
        Some(ServiceManager::SystemdUser) => {
            let socket = std::env::var_os("NOTIFY_SOCKET")
                .ok_or(ServiceError::ManagerUnavailable {
                missing:
                    ovis_terminal_protocol::domain::unix_service::Prerequisite::SystemdUserManager,
            })?;
            service::notify_ready(&socket)
        }
        Some(_) => Ok(()),
    })?;
    Ok(Value::Null)
}

/// Runs `command`; `Ok(None)` means nothing is printed (the holder process itself).
///
/// # Errors
///
/// The typed failure of the verb; nothing is reported as done that did not happen.
pub fn execute(command: &ServiceCommand) -> Result<Option<Value>, ServiceError> {
    match command {
        ServiceCommand::Status(args) => encode(&status(&paths(args)?)?).map(Some),
        ServiceCommand::Install(args) => {
            let paths = paths(&args.session)?;
            let manager = args.manager.protocol();
            let definition = service::install(&paths, manager, &program()?)?;
            let missing = service::prerequisites(Some(manager));
            Ok(Some(
                json!({ "label": paths.session().label(), "manager": manager, "definition": definition, "missing_prerequisites": missing }),
            ))
        }
        ServiceCommand::Uninstall(args) => {
            let paths = paths(&args.session)?;
            let manager = args.manager.protocol();
            let definition = service::uninstall(&paths, manager)?;
            Ok(Some(
                json!({ "label": paths.session().label(), "manager": manager, "removed_definition": definition }),
            ))
        }
        ServiceCommand::Start(args) => start(&paths(&args.session)?, args.manager).map(Some),
        ServiceCommand::Stop(args) => stop(&paths(args)?).map(Some),
        ServiceCommand::Run(args) => run(&paths(&args.session)?, args.manager).map(|_| None),
    }
}
