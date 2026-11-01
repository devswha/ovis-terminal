//! Two remote machines, `alpha` and `bravo`, served by the real machine actor lane over
//! in-process lines, until todo 52 joins two private runtimes over real endpoints.
//!
//! Each remote end is a stand-in runtime: it answers `server.status` with its identity and
//! receives `$/orphaned` with the real [`ovis_terminal_remote::orphans::Inbox`], counting how often a notice was applied.
//! `fixture.machine` freezes it (requests are read and never answered), cuts its line,
//! reconnects it (replaying the orphan outbox) or inspects it; `fixture.remote` stands in
//! for remote dispatch (todo 52) by sending one request through the machine's actor; and
//! `fixture.remote_child` records that a machine holds a child of a local parent, as a
//! remote launch would. `machine.list` is the real lane route; `$/orphanedAck` is answered
//! by the real lane.

pub mod remote;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_agent::tree::TreeService;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::{MachineRecord, MachineState};
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::{DurationMs, OpenEnum};
use ovis_terminal_remote::actors::{Actors, Inbound, Limits, Request};
use ovis_terminal_remote::orphans::{OrphanError, Replayed};
use ovis_terminal_remote::transport::Link;
use ovis_terminal_server::dispatch::lanes::machine_actor::{
    MachineActorLane, MachineList, TreeOutbox, actor_refusal, orphan_refusal,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};

pub(crate) use self::remote::internal;
use self::remote::{PipeLine, Remote};
use super::agents::AgentParts;
use super::handler::{Dispatch, Event};
use super::prompts::Gate;
use super::serve::at;
use crate::error::RunnerError;

/// A fixture machine's line: an in-process stand-in or a spawned transport to a runtime.
pub(crate) type FixtureLine = ovis_terminal_remote::EitherLine<PipeLine, Link>;
pub(crate) type Lane = MachineActorLane<FixtureLine, Event, OperationProducer>;
type Core = ovis_terminal_core::events::Coordinator<Event, OperationProducer>;

/// The fixture machines by name.
const MACHINES: [(&str, &str); 2] = [
    ("alpha", "machine:0191f1a2-0000-7000-8000-0000000000a1"),
    ("bravo", "machine:0191f1a2-0000-7000-8000-0000000000b2"),
];

/// How long one orphan notice may take to be acknowledged.
const DELIVERY: Duration = Duration::from_millis(500);

/// The lane, its remote stand-ins and what records remote children.
#[derive(Debug)]
pub(crate) struct Machines {
    lane: Arc<Lane>,
    remotes: BTreeMap<&'static str, Arc<Remote>>,
    tree: Arc<TreeService>,
    core: Arc<Core>,
    runtime: RuntimeIdentity,
    /// Notifications for the mirror; held so none is counted as dropped.
    _inbound: Mutex<mpsc::Receiver<Inbound>>,
}

fn record((name, id): (&str, &str)) -> Result<MachineRecord, RunnerError> {
    serde_json::from_value(json!({
        "machine_id": id, "name": name, "aliases": [], "state": "connecting",
        "remote_lifetime": "runtime:7", "remote_boot_id": "1", "last_seen_seq": null,
        "version": "1.0.0", "revision": "1",
    }))
    .map_err(at("build a fixture machine record"))
}

/// Registers `machine.list` and attaches a line to each fixture machine.
pub(crate) async fn register(
    dispatch: &mut Dispatch,
    ((status, _), gate): (&AgentParts, &Gate),
    (runtime, core): (RuntimeIdentity, Arc<Core>),
) -> Result<Arc<Machines>, RunnerError> {
    let (actors, inbound) = Actors::new(Limits::new(8, Duration::from_secs(60), 256));
    let services = (Arc::clone(status), Arc::clone(&gate.prompts));
    let tree = Arc::new(TreeService::new(runtime, services));
    for machine in MACHINES {
        actors
            .add(record(machine)?)
            .map_err(at("add a fixture machine"))?;
    }
    let outbox = TreeOutbox::new(Arc::clone(&tree), Arc::clone(&core));
    let lane = MachineActorLane::new(runtime.machine_id(), (Arc::new(actors), outbox), DELIVERY);
    dispatch
        .register_read(MachineList::new(&lane))
        .map_err(at("register machine.list"))?;
    let mut remotes = BTreeMap::new();
    for (name, id) in MACHINES {
        let remote = Remote::new(MachineId::new(id.to_owned()));
        let first_end = remote.connect();
        lane.attach(remote.id(), FixtureLine::First(first_end))
            .await
            .map_err(at("attach a fixture machine"))?;
        remotes.insert(name, remote);
    }
    Ok(Arc::new(Machines {
        lane,
        remotes,
        tree,
        core,
        runtime,
        _inbound: Mutex::new(inbound),
    }))
}

impl Machines {
    /// The machine actor lane every fixture machine, the peer included, is served by.
    pub(crate) const fn lane(&self) -> &Arc<Lane> {
        &self.lane
    }

    /// A stand-in machine, or any machine the actors serve under `name`.
    fn named(&self, name: &str) -> Result<MachineId, BoundaryError> {
        if let Ok(remote) = self.remote(name) {
            return Ok(remote.id().clone());
        }
        let served = self.lane.actors().list().into_iter();
        let mut named = served.filter(|record| record.name.as_str() == name);
        named
            .next()
            .map(|record| record.machine_id)
            .ok_or_else(|| BoundaryError::invalid_params(Some("params.machine")))
    }

    fn remote(&self, name: &str) -> Result<&Arc<Remote>, BoundaryError> {
        self.remotes
            .get(name)
            .ok_or_else(|| BoundaryError::invalid_params(Some("params.machine")))
    }

    async fn inspect(&self, remote: &Remote) -> Result<Value, BoundaryError> {
        let health = self
            .lane
            .actors()
            .health(remote.id())
            .map_err(|error| actor_refusal(&error))?;
        let outbox = self
            .lane
            .pending(remote.id())
            .await
            .map_err(|error| orphan_refusal(&error))?;
        Ok(json!({ "health": health, "outbox": outbox, "remote": remote.counters() }))
    }
}

/// Parameters of `fixture.remote`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteCall {
    machine: String,
    method: String,
    deadline_ms: DurationMs,
}

/// Sends one request through the named machine's actor.
pub(crate) async fn call(
    machines: &Machines,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: RemoteCall = decode_params(request).map_err(BoundaryError::from)?;
    let remote = machines.remote(&params.machine)?;
    let deadline = Duration::from_millis(params.deadline_ms.get());
    let sent = Request::new(&params.method, json!({}), deadline);
    match machines.lane.actors().call(remote.id(), sent).await {
        Ok(Ok(reply)) => Ok(json!({ "reply": reply })),
        Ok(Err(error)) => Ok(json!({ "remote_error": error })),
        Err(error) => Err(actor_refusal(&error)),
    }
}

/// Parameters of `fixture.machine`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct Control {
    machine: String,
    action: Action,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Freeze,
    Cut,
    Reconnect,
    Deliver,
    Inspect,
}

fn replayed(replayed: &Replayed) -> Value {
    json!({
        "applied": replayed.applied, "duplicates": replayed.duplicates,
        "remaining": replayed.remaining,
        "stopped": replayed.stopped.as_ref().map(OrphanError::kind),
    })
}

/// Freezes, cuts, reconnects, delivers to or inspects the named machine.
pub(crate) async fn control(
    machines: &Machines,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: Control = decode_params(request).map_err(BoundaryError::from)?;
    let remote = machines.remote(&params.machine)?;
    let refused = |error: OrphanError| orphan_refusal(&error);
    let replay = match params.action {
        Action::Freeze => {
            remote.freeze();
            None
        }
        Action::Cut => {
            let health = machines.lane.actors().watch_health(remote.id());
            let mut health = health.map_err(|error| actor_refusal(&error))?;
            remote.cut();
            let unreachable = OpenEnum::Known(MachineState::Unreachable);
            let lost = health.wait_for(|state| state.state == unreachable);
            tokio::time::timeout(Duration::from_secs(10), lost)
                .await
                .map_err(internal)?
                .map_err(internal)?;
            None
        }
        Action::Reconnect => {
            let fresh_end = remote.connect();
            Some(
                machines
                    .lane
                    .attach(remote.id(), FixtureLine::First(fresh_end))
                    .await
                    .map_err(refused)?,
            )
        }
        Action::Deliver => Some(machines.lane.deliver(remote.id()).await.map_err(refused)?),
        Action::Inspect => None,
    };
    let mut inspected = machines.inspect(remote).await?;
    if let (Some(replay), Some(object)) = (replay, inspected.as_object_mut()) {
        object.insert("replayed".to_owned(), replayed(&replay));
    }
    Ok(inspected)
}

/// Parameters of `fixture.remote_child`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteChild {
    parent: ParentRef,
    machine: String,
}

/// Records, in one operation, that the named machine holds a child of a live local parent.
pub(crate) async fn remote_child(
    machines: &Arc<Machines>,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: RemoteChild = decode_params(request).map_err(BoundaryError::from)?;
    let machine = machines.named(&params.machine)?;
    let fixture = Arc::clone(machines);
    tokio::task::spawn_blocking(move || {
        remote::link(
            &fixture.tree,
            (&fixture.core, fixture.runtime),
            (params.parent, machine),
        )
    })
    .await
    .map_err(internal)?
}

/// `$/orphanedAck`, answered by the lane.
pub(crate) async fn acknowledge(
    machines: &Machines,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    machines.lane.acknowledge(request.params().clone()).await
}
