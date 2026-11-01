//! Machine-scoped lanes: credentials, `server.stop`, documents, plugins, machine
//! identity and the holder service description.

use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_server::dispatch::lanes::credential_rotation::{
    CredentialLane, RevokeCredential, RotateCredential,
};
use ovis_terminal_server::dispatch::lanes::machine_identity::{
    MachineAdd, MachineGet, MachineLane, MachineRename,
};
use ovis_terminal_server::dispatch::lanes::{documents_retention, plugins};
use ovis_terminal_services::plugins::Plugins;

use super::{Dispatch, Hosting, stage};
use crate::assembly::AssemblyError;

/// Bound on one remote machine handshake.
const HANDSHAKE_BOUND: Duration = Duration::from_secs(10);

/// Registers the machine-scoped lanes.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
) -> Result<(), AssemblyError> {
    let identity = hosting.identity;
    let machine = identity.machine_id();
    let credentials = CredentialLane::new(
        Arc::clone(&hosting.authority),
        machine,
        Arc::new(crate::assembly::credentials::ActorWrites(
            hosting.actor.clone(),
        )),
    );
    dispatch
        .register(RotateCredential::new(&credentials))
        .map_err(stage("register credential.rotate"))?;
    dispatch
        .register(RevokeCredential::new(&credentials))
        .map_err(stage("register credential.revoke"))?;
    crate::assembly::restart::register(dispatch, Arc::clone(&hosting.control), machine)?;
    documents_retention::register(dispatch, identity).map_err(stage("register documents"))?;
    let root = hosting.state.join("plugins");
    std::fs::create_dir_all(&root).map_err(stage("create the plugin root"))?;
    let (list, invoke, run) = plugins::routes(&Arc::new(Plugins::new(root)), machine);
    dispatch
        .register(list)
        .map_err(stage("register plugin.list"))?;
    dispatch
        .register(invoke)
        .map_err(stage("register plugin.invoke"))?;
    dispatch
        .register(run)
        .map_err(stage("register plugin.run_get"))?;
    let machines = MachineLane::new(machine, HANDSHAKE_BOUND);
    dispatch
        .register(MachineGet::new(&machines))
        .map_err(stage("register machine.get"))?;
    dispatch
        .register(MachineRename::new(&machines))
        .map_err(stage("register machine.rename"))?;
    dispatch
        .register(MachineAdd::new(&machines))
        .map_err(stage("register machine.add"))?;
    super::service::register(dispatch, machine)
}
