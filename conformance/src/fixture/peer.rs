//! The second private runtime (plan todo 52) and the machines a remote scenario adds.
//!
//! The peer is this runner re-run as `fixture --peer` in its own root, with its own
//! authority, store and machine identity. It is reached only through the real
//! `machine.add` route, whose argv transport runs the binary under test as
//! `ovis-terminal connect` against the peer's socket, and it is addressed through the real
//! remote dispatch. `fixture.peer` stops, resumes, crashes or restarts that process;
//! `fixture.lost_machine` adds a machine whose line delivered events and then ended.

pub mod lost;
pub mod orphans;
mod process;
pub mod ssh;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use ovis_terminal_authz::Authority;
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_remote::identity::{Credential, Trust};
use ovis_terminal_remote::orphans::{OrphanError, Replayed};
use ovis_terminal_remote::transport::TransportSpec;
use ovis_terminal_server::dispatch::lanes::machine_identity::{
    MachineAdd, MachineGet, MachineLane, MachineRename,
};
use ovis_terminal_server::dispatch::remote::{MachineAddAttached, RemoteRoutes, reconnect};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

pub(crate) use self::lost::{LOST_MACHINE, lost_machine};
use self::process::Process;
use super::Role;
use super::handler::Dispatch;
use super::machines::{FixtureLine, Lane, Machines, internal};
use super::serve::at;
use crate::error::RunnerError;

/// Machine identity of the second private runtime.
pub(crate) const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-0000000000c3";

/// Whether this process serves as the second private runtime; set once at serve time.
static PEER: AtomicBool = AtomicBool::new(false);

/// Records which runtime this fixture process is, before it reports any identity.
pub(crate) fn assume(role: &Role) {
    PEER.store(matches!(role, Role::Peer), Ordering::SeqCst);
}

/// Machine identity this fixture process reports in hello.
pub(crate) fn machine() -> &'static str {
    if PEER.load(Ordering::SeqCst) {
        MACHINE
    } else {
        super::serve::MACHINE
    }
}

/// The unadvertised control of the peer process.
pub(crate) const CONTROL: &str = "fixture.peer";

/// Bound on one handshake and on any one awaited state change.
const BOUND: Duration = Duration::from_secs(20);

/// Bound on one remote call; a stalled machine fails its call at this deadline.
const REMOTE_DEADLINE: Duration = Duration::from_secs(10);

/// The peer process, started on first use, and what reaches it.
#[derive(Debug)]
pub(crate) struct PeerSide {
    runner: PathBuf,
    binary: PathBuf,
    root: PathBuf,
    started: AtomicU32,
    process: Mutex<Option<Process>>,
    lane: Arc<Lane>,
}

/// The peer side (runtime under test only), the service lanes and the remote dispatch
/// routes through the fixture's machine actors, each call bounded.
pub(crate) fn compose(
    dispatch: &mut Dispatch,
    (root, role): (&Path, &Role),
    (machines, authority, runtime): (
        &Machines,
        &Arc<Authority<RegistryIdentities>>,
        RuntimeIdentity,
    ),
) -> Result<Composed, RunnerError> {
    let peer = register(dispatch, (root, role), (machines, runtime))?;
    super::services::register(dispatch, root, runtime)?;
    let worktrees = super::worktrees::serve(root, authority, runtime)?;
    let actors = Arc::clone(machines.lane().actors());
    let routes = RemoteRoutes::new(
        (actors, Arc::clone(authority)),
        runtime.machine_id(),
        REMOTE_DEADLINE,
    );
    Ok((peer, routes, worktrees))
}

/// The peer side, the remote routes and the worktree lane a fixture runtime serves.
pub(crate) type Composed = (
    Option<Arc<PeerSide>>,
    RemoteRoutes<FixtureLine>,
    Arc<super::worktrees::WorktreeSide>,
);

/// Stops the peer process, if this fixture started one.
pub(crate) async fn stop(peer: Option<Arc<PeerSide>>) -> Result<(), RunnerError> {
    match peer {
        Some(peer) => peer.stop().await,
        None => Ok(()),
    }
}

/// For the runtime under test: registers the real `machine.add` (attaching each added
/// machine to its actor), `machine.get` and `machine.rename`, and prepares the peer
/// runtime. The peer itself adds no machines.
fn register(
    dispatch: &mut Dispatch,
    (root, role): (&Path, &Role),
    (machines, runtime): (&Machines, RuntimeIdentity),
) -> Result<Option<Arc<PeerSide>>, RunnerError> {
    let Role::Local {
        binary: Some(binary),
    } = role
    else {
        return Ok(None);
    };
    let identity = MachineLane::new(runtime.machine_id(), BOUND);
    let add = MachineAdd::new(&identity);
    let attached = MachineAddAttached::new((add, &identity), machines.lane(), FixtureLine::Second);
    dispatch
        .register(attached)
        .map_err(at("register machine.add"))?;
    let get = MachineGet::new(&identity);
    dispatch.register(get).map_err(at("register machine.get"))?;
    let rename = MachineRename::new(&identity);
    dispatch
        .register(rename)
        .map_err(at("register machine.rename"))?;
    Ok(Some(Arc::new(PeerSide {
        runner: std::env::current_exe().map_err(at("locate the runner"))?,
        binary: binary.clone(),
        root: root.to_path_buf(),
        started: AtomicU32::new(0),
        process: Mutex::new(None),
        lane: Arc::clone(machines.lane()),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    action: Action,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Describe,
    Stop,
    Resume,
    Crash,
    Restart,
    Deliver,
}

impl PeerSide {
    fn machine() -> MachineId {
        MachineId::new(MACHINE.to_owned())
    }

    /// The argv transport that reaches the process: the binary under test as a stdio bridge.
    fn transport(&self, process: &Process) -> Vec<String> {
        vec![
            "/usr/bin/env".to_owned(),
            format!("OVIS_TERMINAL_SOCKET={}", process.socket.display()),
            self.binary.display().to_string(),
            "connect".to_owned(),
        ]
    }

    async fn spawn(&self) -> Result<Process, BoundaryError> {
        let index = self.started.fetch_add(1, Ordering::SeqCst);
        let dir = self.root.join(format!("peer-{index}"));
        let process = Process::spawn(&self.runner, &dir).await.map_err(internal)?;
        let shim = (self.binary.as_path(), process.socket.as_path());
        ssh::install(&self.root.join("bin"), shim).map_err(internal)?;
        Ok(process)
    }

    fn health(&self) -> Result<Value, BoundaryError> {
        let health = self.lane.actors().health(&Self::machine());
        serde_json::to_value(health.map_err(internal)?).map_err(internal)
    }

    /// Waits until the peer's actor holds a call on its line, kills the process, then waits
    /// until the actor has seen the line end.
    async fn crash(&self, process: &mut Process) -> Result<Value, BoundaryError> {
        let mut health = self
            .lane
            .actors()
            .watch_health(&Self::machine())
            .map_err(internal)?;
        let holding = health.wait_for(|health| health.in_flight.get() >= 1);
        tokio::time::timeout(BOUND, holding)
            .await
            .map_err(internal)?
            .map_err(internal)?;
        process.crash().await?;
        let unreachable = OpenEnum::Known(MachineState::Unreachable);
        let lost = health.wait_for(|health| health.state == unreachable);
        tokio::time::timeout(BOUND, lost)
            .await
            .map_err(internal)?
            .map_err(internal)?;
        Ok(json!({"crashed": true, "health": self.health()?}))
    }

    /// A fresh process proves the same machine over a fresh challenge and takes the line.
    async fn restart(&self, slot: &mut Option<Process>) -> Result<Value, BoundaryError> {
        if let Some(previous) = slot.take() {
            previous.stop().await.map_err(internal)?;
        }
        let process = self.spawn().await?;
        let spec = TransportSpec::Argv(self.transport(&process));
        let credential = Credential::new(process.bearer.clone());
        let trust = Trust::required(Some(Self::machine()), credential).map_err(internal)?;
        let replayed = reconnect(&self.lane, (&spec, &trust), (BOUND, FixtureLine::Second)).await;
        let replayed = replayed.map_err(|error| {
            BoundaryError::domain(error.kind().to_owned().into(), false).with_hint(error.hint())
        })?;
        *slot = Some(process);
        self.owed(Ok(replayed)).await
    }

    /// The replay outcome, the notices still owed to the peer and its health.
    async fn owed(&self, replayed: Result<Replayed, OrphanError>) -> Result<Value, BoundaryError> {
        let mut owed = orphans::report(&self.lane, &Self::machine(), replayed).await?;
        if let Some(object) = owed.as_object_mut() {
            object.insert("health".to_owned(), self.health()?);
        }
        Ok(owed)
    }

    async fn control(&self, action: Action) -> Result<Value, BoundaryError> {
        let mut slot = self.process.lock().await;
        if slot.is_none() && !matches!(action, Action::Restart) {
            *slot = Some(self.spawn().await?);
        }
        let process = slot.as_mut();
        let result = match (action, process) {
            (Action::Describe, Some(process)) => Ok(json!({
                "machine_id": MACHINE, "argv": self.transport(process), "ssh": ssh::DESTINATION,
                "credential": process.bearer,
            })),
            (Action::Stop, Some(process)) => {
                process.pause().await.map(|()| json!({"stopped": true}))
            }
            (Action::Resume, Some(process)) => process.resume().map(|()| json!({"stopped": false})),
            (Action::Crash, Some(process)) => self.crash(process).await,
            (Action::Restart, _) => self.restart(&mut slot).await,
            (Action::Deliver, Some(_)) => {
                let replayed = self.lane.deliver(&Self::machine()).await;
                self.owed(replayed).await
            }
            (_, None) => Err(internal("the peer runtime is not running")),
        };
        drop(slot);
        result
    }

    /// Stops the peer process, if one runs.
    async fn stop(&self) -> Result<(), RunnerError> {
        let running = self.process.lock().await.take();
        match running {
            Some(process) => process.stop().await,
            None => Ok(()),
        }
    }
}

/// `fixture.peer`: describes, stops, resumes, crashes or restarts the peer runtime.
pub(crate) async fn control(
    peer: Option<&PeerSide>,
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: Control = decode_params(request).map_err(BoundaryError::from)?;
    let peer = peer.ok_or_else(|| internal("this fixture serves no peer runtime"))?;
    peer.control(params.action).await
}
