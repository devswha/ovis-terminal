//! Remote machines (T52): `machine.add` attaches each proven machine's link to its own
//! actor, `machine.get`/`machine.rename` read and rename identities, `machine.list` reads
//! the actor roster, and the returned routes forward a machine-targeted call to its
//! actor. The daemon keeps no mirror of remote events, so the actors' inbound
//! notifications are drained and dropped.

use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_agent::tree::TreeService;
use ovis_terminal_remote::actors::{Actors, Limits};
use ovis_terminal_remote::transport::Link;
use ovis_terminal_server::dispatch::lanes::machine_actor::{
    MachineActorLane, MachineList, TreeOutbox,
};
use ovis_terminal_server::dispatch::lanes::machine_identity::{
    MachineAdd, MachineGet, MachineLane, MachineRename,
};
use ovis_terminal_server::dispatch::remote::{MachineAddAttached, RemoteRoutes};

use super::{Dispatch, DispatchLog, Hosting, stage};
use crate::assembly::AssemblyError;

/// Bound on one remote machine handshake.
const HANDSHAKE_BOUND: Duration = Duration::from_secs(10);
/// Bound on one forwarded remote call.
const REMOTE_DEADLINE: Duration = Duration::from_secs(30);
/// How long one orphan notice may take to be acknowledged.
const DELIVERY: Duration = Duration::from_secs(5);

/// The routes remote calls take.
pub(in crate::assembly) type Routes = RemoteRoutes<Link>;

/// Registers the machine lanes and returns the routes of the machine actors.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
    log: &Arc<DispatchLog>,
) -> Result<Routes, AssemblyError> {
    let machine = hosting.identity.machine_id();
    let (actors, mut inbound) = Actors::new(Limits::new(8, Duration::from_secs(60), 256));
    tokio::spawn(async move { while inbound.recv().await.is_some() {} });
    let services = (
        Arc::clone(hosting.agents.status()),
        Arc::clone(hosting.agents.prompts()),
    );
    let tree = Arc::new(TreeService::new(hosting.identity, services));
    let outbox = TreeOutbox::new(tree, Arc::clone(log));
    let actors = Arc::new(actors);
    let lane = MachineActorLane::new(machine, (Arc::clone(&actors), outbox), DELIVERY);
    dispatch
        .register_read(MachineList::new(&lane))
        .map_err(stage("register machine.list"))?;
    let identities = MachineLane::new(machine, HANDSHAKE_BOUND);
    let add = MachineAdd::new(&identities);
    dispatch
        .register(MachineAddAttached::new(
            (add, &identities),
            &lane,
            std::convert::identity,
        ))
        .map_err(stage("register machine.add"))?;
    dispatch
        .register(MachineGet::new(&identities))
        .map_err(stage("register machine.get"))?;
    dispatch
        .register(MachineRename::new(&identities))
        .map_err(stage("register machine.rename"))?;
    Ok(RemoteRoutes::new(
        (actors, Arc::clone(&hosting.authority)),
        machine,
        REMOTE_DEADLINE,
    ))
}
