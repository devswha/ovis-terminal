//! The template launch lane over the fixture's templates and a recording launcher.
//!
//! The launcher stands in for the holder until `terminal.run` is assembled on the
//! runtime (todos 31 and 42): it starts no process. It records each start's literal
//! argv, environment and parent in `effects/spawn-<id>.json`, registers the terminal
//! under its name with a process token, and records each submission written to it in
//! `effects/input-<id>.json`. Declared status reaches the terminal's wait hub from the
//! committed `status.changed` events of the real `status.report` lane, through a
//! subscription installed before any launch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner, StatusService};
use ovis_terminal_authz::Authority;
use ovis_terminal_core::events::{Delivery, ResourceId};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::registry::{
    ProcessGeneration, RegistryIdentities, ResourceName, RuntimeIdentity, TerminalId,
};
use ovis_terminal_protocol::domain::template_launch::{LaunchedTerminal, started_process};
use ovis_terminal_protocol::schema::ids::{
    MachineId, ProcessToken, TerminalAddress, TerminalId as WireTerminalId,
};
use ovis_terminal_protocol::schema::scalar::{EventSeq, Incarnation, TimestampMs};
use ovis_terminal_server::dispatch::DispatchError;
use ovis_terminal_server::dispatch::lanes::template_launch::{
    Boundary, Launch, Launcher, Reservation,
};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use serde_json::{Value, json};

use super::agents::Directory;
use super::peer::machine;
use super::template_lane::{duplicate_name, feed, hub};

/// Identity of the first launched terminal; earlier ids belong to the static terminals.
const FIRST_LAUNCHED: u64 = 100;

#[derive(Debug)]
struct Launched {
    waits: ovis_terminal_core::waits::Waits,
    leases: TerminalLeases,
    writes: Vec<Value>,
}

/// Starts nothing; records starts and writes, and feeds declared status to wait hubs.
#[derive(Debug)]
pub(crate) struct RecordingLauncher {
    effects: PathBuf,
    cwd: PathBuf,
    authority: Arc<Authority<RegistryIdentities>>,
    identity: RuntimeIdentity,
    agents: (Arc<StatusService>, Arc<Directory>),
    terminals: Arc<Mutex<BTreeMap<TerminalId, Launched>>>,
    next: Mutex<u64>,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
}

impl RecordingLauncher {
    pub(crate) fn new(
        (effects, cwd): (&Path, &Path),
        (authority, identity): (Arc<Authority<RegistryIdentities>>, RuntimeIdentity),
        (agents, events): (
            (Arc<StatusService>, Arc<Directory>),
            Delivery<AgentStatusEvent>,
        ),
    ) -> Self {
        let terminals = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let pump = {
            let (terminals, stop) = (Arc::clone(&terminals), Arc::clone(&stop));
            std::thread::spawn(move || pump(&events, &terminals, &stop))
        };
        Self {
            effects: effects.to_path_buf(),
            cwd: cwd.to_path_buf(),
            authority,
            identity,
            agents,
            terminals,
            next: Mutex::new(FIRST_LAUNCHED),
            stop,
            pump: Some(pump),
        }
    }

    fn launched<T>(
        &self,
        terminal: TerminalId,
        use_it: impl FnOnce(&mut Launched) -> T,
    ) -> Result<T, DispatchError> {
        let mut terminals = self.terminals.lock().map_err(|_| DispatchError::Closed)?;
        terminals
            .get_mut(&terminal)
            .map(use_it)
            .ok_or(DispatchError::Closed)
    }

    fn record(&self, file: String, value: &Value) -> Result<(), DispatchError> {
        std::fs::write(self.effects.join(file), serde_json::to_vec(value)?)?;
        Ok(())
    }
}

/// Delivers each committed root status of a launched terminal to its wait hub. The
/// receive bound only paces the stop check; delivery never depends on it.
fn pump(
    events: &Delivery<AgentStatusEvent>,
    terminals: &Mutex<BTreeMap<TerminalId, Launched>>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::SeqCst) {
        let Ok(event) = events.recv_timeout(Duration::from_millis(250)) else {
            continue;
        };
        let AgentStatusEvent::StatusChanged(snapshot) = &event.data else {
            continue;
        };
        let terminals = terminals.lock().unwrap_or_else(PoisonError::into_inner);
        for resource in &event.affected {
            if let ResourceId::Terminal(id) = resource
                && let Some(launched) = terminals.get(id)
            {
                feed(&launched.waits, snapshot);
            }
        }
    }
}

impl Drop for RecordingLauncher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(pump) = self.pump.take() {
            // The pump only forwards status; a panic in it has nothing left to report.
            drop(pump.join());
        }
    }
}

impl Launcher for RecordingLauncher {
    fn reserve(&self, launch: Launch<'_>) -> Result<Reservation, DispatchError> {
        if let Some(name) = &launch.params.name
            && self.agents.1.named(name.as_str())
        {
            return Err(duplicate_name());
        }
        let id = {
            let mut next = self.next.lock().map_err(|_| DispatchError::Closed)?;
            let id = *next;
            *next = id.checked_add(1).ok_or(DispatchError::Closed)?;
            id
        };
        let terminal = TerminalId::try_from(id).map_err(|_| DispatchError::Closed)?;
        let (waits, leases) = hub(terminal, (&self.authority, self.identity.machine_id()))?;
        let process = ProcessGeneration::new(1).map_err(|_| DispatchError::Closed)?;
        let launched = Launched {
            waits: waits.clone(),
            leases,
            writes: Vec::new(),
        };
        self.terminals
            .lock()
            .map_err(|_| DispatchError::Closed)?
            .insert(terminal, launched);
        Ok(Reservation::new(terminal, process, waits))
    }

    fn start(
        &self,
        reservation: &Reservation,
        launch: Launch<'_>,
    ) -> Result<LaunchedTerminal, DispatchError> {
        let terminal = reservation.terminal;
        let token = format!("fixture-launch-token-{terminal}");
        let owner = ProcessOwner::new(terminal, Incarnation::new(reservation.process.get()));
        let (service, directory) = &self.agents;
        service
            .tokens()
            .bind(&ProcessToken::new(token.clone()), owner)
            .map_err(|_| DispatchError::Closed)?;
        let name = launch
            .params
            .name
            .as_ref()
            .map(|name| name.as_str().to_owned());
        if let Some(name) = &name {
            directory.add(
                terminal,
                ResourceName::new(name).map_err(|_| DispatchError::Closed)?,
            );
        }
        let mut env = launch.plan.env.clone();
        env.insert("OVIS_TERMINAL_PROCESS_TOKEN".to_owned(), token);
        let cwd = launch
            .params
            .cwd
            .clone()
            .unwrap_or_else(|| self.cwd.display().to_string());
        self.record(format!("spawn-{terminal}.json"), &json!({
            "argv": launch.plan.argv, "env": env, "cwd": cwd, "name": name, "parent": launch.params.parent,
        }))?;
        let process = started_process(
            Incarnation::new(reservation.process.get()),
            (launch.plan.argv.clone(), Some(cwd)),
            TimestampMs::new(0),
        );
        let wire = WireTerminalId::new(terminal.to_string());
        let address =
            TerminalAddress::new(format!("local/{}", name.as_deref().unwrap_or("unnamed")));
        let identity = (wire, address, MachineId::new(machine().to_owned()));
        Ok(LaunchedTerminal::new(
            identity,
            (process, EventSeq::new(0)),
            launch.params,
        ))
    }

    fn boundary(&self, terminal: TerminalId) -> Result<Boundary, DispatchError> {
        let geometry = Geometry::new(80, 24).map_err(|_| DispatchError::Closed)?;
        let modes = ModeSnapshot::capture(&Vt::new(VtConfig::new(geometry, HistoryLimit::new(10))));
        let process = ProcessGeneration::new(1).map_err(|_| DispatchError::Closed)?;
        self.launched(terminal, |launched| {
            Boundary::new(launched.leases.view(), modes, process)
        })
    }

    fn write(&self, terminal: TerminalId, bytes: &[u8]) -> Result<EventSeq, DispatchError> {
        let writes = self.launched(terminal, |launched| {
            let seq = u64::try_from(launched.writes.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            launched.writes.push(
                json!({"queued_seq": seq.to_string(), "text": String::from_utf8_lossy(bytes)}),
            );
            (seq, Value::from(launched.writes.clone()))
        })?;
        self.record(format!("input-{terminal}.json"), &writes.1)?;
        Ok(EventSeq::new(writes.0))
    }
}
