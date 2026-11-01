//! `service.describe`: the holder service registration of this user's default session,
//! from the definitions the runtime wrote and read-only manager queries. It never installs,
//! starts or repairs a service, and it is an unrecorded read.

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_holder::service::unix::{
    ServiceEnvironment, ServiceError, ServicePaths, ServiceSession,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_server::dispatch::lanes::unix_service::describe;
use ovis_terminal_server::dispatch::{Access, DispatchError, Read};
use ovis_terminal_store::Snapshot;
use serde_json::Value;

use super::{Dispatch, stage};
use crate::assembly::AssemblyError;
use crate::assembly::error::{internal, refusal};

/// The session `ovis-terminal service` addresses when `--session` is omitted.
const DEFAULT_SESSION: &str = "default";

/// Describes the default session's registration as the process environment resolves it.
#[derive(Debug)]
struct Describe {
    machine: MachineId,
}

/// The catalogued kind of a describe failure: holder tokens the error catalog does not
/// publish travel as `unavailable`, with the holder's own hint.
fn refused(error: &ServiceError) -> BoundaryError {
    let kind = match error.kind() {
        "service_unsupported" => "service_unsupported",
        _ => "unavailable",
    };
    refusal(kind, error.hint())
}

impl Read for Describe {
    const NAME: &'static str = "service.describe";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::Read,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        ))
    }

    fn read(&self, _: Self::Params, _: &Snapshot) -> Result<Value, BoundaryError> {
        let session = ServiceSession::parse(DEFAULT_SESSION).map_err(|error| refused(&error))?;
        let paths = ServicePaths::resolve(ServiceEnvironment::from_process(), session)
            .map_err(|error| refused(&error))?;
        let described = describe(&[paths]).map_err(|error| refused(&error))?;
        serde_json::to_value(described).map_err(internal("encode service.describe"))
    }
}

/// Registers `service.describe` for `machine`.
pub(super) fn register(dispatch: &mut Dispatch, machine: MachineId) -> Result<(), AssemblyError> {
    dispatch
        .register_read(Describe { machine })
        .map_err(stage("register service.describe"))
}
