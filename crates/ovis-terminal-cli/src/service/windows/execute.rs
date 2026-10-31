//! Running one Windows service verb.

use std::time::Duration;

use ovis_terminal_holder::service::windows::{
    self as service, HostConfig, HostError, HostPaths, HostSession, ServiceDefinition, pipe,
};
use ovis_terminal_protocol::domain::unix_service::Observed;
use ovis_terminal_protocol::domain::windows_service::{ServiceAccount, WindowsServiceStatus};
use serde_json::{Value, json};

use super::args::{HostArgs, ServiceCommand};

/// Bound on the manager reaching running or stopped.
const STATE_BOUND: Duration = Duration::from_secs(60);
const PASSWORD_VAR: &str = "OVIS_TERMINAL_SERVICE_PASSWORD";

fn paths(args: &HostArgs) -> Result<HostPaths, HostError> {
    let session = HostSession::parse(&args.session)?;
    match &args.state_dir {
        Some(state) => HostPaths::new(session, state.clone()),
        None => HostPaths::resolve(session, std::env::var_os("LOCALAPPDATA")),
    }
}

fn owner(args: &HostArgs) -> Result<String, HostError> {
    args.owner_sid.clone().map_or_else(
        || {
            pipe::current_user_sid()
                .map_err(|source| HostError::os("read this account's SID", source))
        },
        Ok,
    )
}

fn encode(value: &impl serde::Serialize) -> Result<Value, HostError> {
    serde_json::to_value(value).map_err(|source| HostError::Io {
        operation: "encode the result",
        source: source.into(),
    })
}

/// One observation plus the manager's view; it changes nothing.
fn status(paths: &HostPaths, owner: &str) -> Result<WindowsServiceStatus, HostError> {
    let (observed, placement) = service::observe(paths, owner)?;
    let installed = service::installation(paths, owner)?;
    Ok(
        WindowsServiceStatus::new(paths.session().label(), observed)
            .with_host(installed, placement),
    )
}

fn install(args: &super::args::InstallArgs) -> Result<Value, HostError> {
    let account = ServiceAccount::parse(&args.account).map_err(HostError::Name)?;
    let password = std::env::var(PASSWORD_VAR).map_err(|_| HostError::PasswordRequired)?;
    let owner = service::account_sid(&account)?;
    let paths = paths(&args.host)?;
    let program = std::env::current_exe()
        .map_err(|source| HostError::os("locate this executable", source))?;
    let definition = ServiceDefinition::new(&paths, &program, (&owner, args.placement.protocol()))?;
    service::install(&definition, &account, &password)?;
    Ok(json!({
        "label": definition.name,
        "manager": "windows_service",
        "account": account.as_str(),
        "owner_sid": owner,
        "command_line": definition.command_line,
        "placement": args.placement.protocol(),
    }))
}

fn start(args: &HostArgs) -> Result<Value, HostError> {
    let (paths, owner) = (paths(args)?, owner(args)?);
    service::start(&paths, &owner, STATE_BOUND)?;
    let started = status(&paths, &owner)?;
    match &started.observed {
        Observed::Running { .. } => encode(&started),
        Observed::StaleEndpoint { reason, .. } => Err(HostError::Stale { reason: *reason }),
        Observed::Stopped => Err(HostError::NotRunning),
        _ => Err(HostError::Unreachable),
    }
}

fn stop(args: &HostArgs) -> Result<Value, HostError> {
    let (paths, owner) = (paths(args)?, owner(args)?);
    let ended = match service::observe(&paths, &owner)?.0 {
        Observed::Running { record }
        | Observed::Unreachable {
            record: Some(record),
        } => Some(record.boot_proof),
        _ => None,
    };
    let installed = service::stop(&paths, &owner, STATE_BOUND)?;
    let after = status(&paths, &owner)?;
    Ok(json!({
        "label": paths.session().label(),
        "ended_holder": ended,
        "installed": installed,
        "state": after.observed.state(),
    }))
}

/// Runs `command`; `Ok(None)` means nothing is printed (the service process itself).
///
/// # Errors
///
/// The typed failure of the verb; nothing is reported as done that did not happen.
pub fn execute(command: &ServiceCommand) -> Result<Option<Value>, HostError> {
    match command {
        ServiceCommand::Status(args) => encode(&status(&paths(args)?, &owner(args)?)?).map(Some),
        ServiceCommand::Install(args) => install(args).map(Some),
        ServiceCommand::Uninstall(args) => {
            let paths = paths(args)?;
            service::uninstall(&paths, &owner(args)?)?;
            Ok(Some(
                json!({ "label": paths.session().label(), "removed": true }),
            ))
        }
        ServiceCommand::Start(args) => start(args).map(Some),
        ServiceCommand::Stop(args) => stop(args).map(Some),
        ServiceCommand::Run(args) => {
            service::run_service(HostConfig::new(
                paths(&args.host)?,
                args.placement.protocol(),
            ))?;
            Ok(None)
        }
    }
}
