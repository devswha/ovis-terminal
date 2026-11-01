//! Which machine a request is for, resolved once from its parameters.

use ovis_terminal_authz::Action;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_remote::actors::Line;
use serde_json::Value;

use super::RemoteRoutes;

const ADDRESS: &str = "params.target.address";
const MACHINE: &str = "params.machine";

/// A request for another machine, with the parameters that machine receives.
#[derive(Debug)]
pub(super) struct Remote {
    pub(super) machine: MachineId,
    pub(super) params: Value,
}

#[derive(Debug)]
pub(super) enum Route {
    Local,
    Remote(Remote),
}

/// The action a remote request needs on this runtime's machine scope.
pub(super) fn action(method: &str, mutates: bool) -> Action {
    if method.starts_with("input.") {
        Action::Input
    } else if method == "terminal.run" {
        Action::Launch
    } else if mutates {
        Action::Administer
    } else {
        Action::Read
    }
}

/// Routes `params` to the local handler or to exactly one known remote machine.
pub(super) fn of<L: Line>(
    params: &Value,
    routes: &RemoteRoutes<L>,
) -> Result<Route, BoundaryError> {
    if let Some(address) = params.pointer("/target/address").and_then(Value::as_str) {
        let Some((qualifier, rest)) = address.split_once('/') else {
            return Ok(Route::Local);
        };
        let Some(machine) = resolve(routes, qualifier, ADDRESS)? else {
            return Ok(Route::Local);
        };
        let mut forwarded = params.clone();
        if let Some(slot) = forwarded.pointer_mut("/target/address") {
            *slot = Value::String(rest.to_owned());
        }
        return Ok(Route::Remote(Remote {
            machine,
            params: forwarded,
        }));
    }
    let Some(reference) = params.get("machine").filter(|machine| machine.is_object()) else {
        return Ok(Route::Local);
    };
    let reference: MachineRef = serde_json::from_value(reference.clone())
        .map_err(|_| BoundaryError::invalid_params(Some(MACHINE)))?;
    let named = match &reference {
        MachineRef::MachineId(id) => id.as_str().to_owned(),
        MachineRef::Name(name) => name.as_str().to_owned(),
        _ => return Err(BoundaryError::invalid_params(Some(MACHINE))),
    };
    let Some(machine) = resolve(routes, &named, MACHINE)? else {
        return Ok(Route::Local);
    };
    let mut forwarded = params.clone();
    if let Some(object) = forwarded.as_object_mut() {
        object.remove("machine");
    }
    Ok(Route::Remote(Remote {
        machine,
        params: forwarded,
    }))
}

/// `None` for this runtime's own machine; an unknown machine is refused, never served
/// locally in its place.
fn resolve<L: Line>(
    routes: &RemoteRoutes<L>,
    named: &str,
    field: &'static str,
) -> Result<Option<MachineId>, BoundaryError> {
    if named == routes.local.to_string() {
        return Ok(None);
    }
    routes
        .actors
        .list()
        .into_iter()
        .find(|record| {
            record.machine_id.as_str() == named
                || record.name.as_str() == named
                || record.aliases.iter().any(|alias| alias.as_str() == named)
        })
        .map(|record| Some(record.machine_id))
        .ok_or_else(|| super::refusal::unknown_machine(field))
}
