//! T35's template launch lane over the daemon's runtime: `template.list` and
//! `template.get` as unrecorded reads of `<state>/templates`, `terminal.run` with a template
//! or a first message through T35's `RunTerminal`, and `terminal.first_message`. The
//! launcher runs each step on the runtime actor; the caller's session reaches it by op id
//! (the served handler records it before the dispatcher runs the launch).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use ovis_terminal_authz::Session;
use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::domain::template_launch::LaunchedTerminal;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::dispatch::DispatchError;
use ovis_terminal_server::dispatch::lanes::template_launch::{
    Boundary, GetFirstMessage, GetTemplate, Launch, Launcher, ListTemplates, Reservation,
    RunTerminal, TemplateLaunchLane,
};
use ovis_terminal_services::templates::TemplateCatalog;

use super::{Dispatch, Hosting, stage};
use crate::assembly::AssemblyError;
use crate::assembly::actor::Actor;

#[path = "templates/launched.rs"]
mod launched;

/// Callers' sessions of template launches in flight: by op id until the launch reserved
/// its terminal, then by terminal for the first message's write.
#[derive(Debug, Clone, Default)]
pub(in crate::assembly) struct LaunchSessions(Arc<Mutex<Sessions>>);

#[derive(Debug, Default)]
struct Sessions {
    by_op: HashMap<String, Arc<Session>>,
    by_terminal: HashMap<TerminalId, Arc<Session>>,
}

impl LaunchSessions {
    /// Records the caller of the launch `op_id` before the dispatcher runs it.
    pub(in crate::assembly) fn record(&self, op_id: &str, session: Arc<Session>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .by_op
            .insert(op_id.to_owned(), session);
    }

    fn of_op(&self, op_id: &str) -> Result<Arc<Session>, DispatchError> {
        let sessions = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sessions
            .by_op
            .get(op_id)
            .cloned()
            .ok_or(DispatchError::Closed)
    }

    fn reserved(&self, op_id: &str, terminal: TerminalId) {
        let mut sessions = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(session) = sessions.by_op.remove(op_id) {
            sessions.by_terminal.insert(terminal, session);
        }
    }

    fn of_terminal(&self, terminal: TerminalId) -> Result<Arc<Session>, DispatchError> {
        let sessions = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sessions
            .by_terminal
            .get(&terminal)
            .cloned()
            .ok_or(DispatchError::Closed)
    }
}

/// The static token of a runtime refusal's kind, which the dispatcher's error carries. The
/// runtime refuses with a finite set of kinds, so each distinct one is stored once.
fn interned(kind: &str) -> &'static str {
    static KINDS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut kinds = KINDS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = kinds.get(kind) {
        return known;
    }
    let stored: &'static str = Box::leak(kind.to_owned().into_boxed_str());
    kinds.insert(stored);
    stored
}

fn domain(error: BoundaryError) -> DispatchError {
    let (kind, hint) = (interned(error.kind()), error.hint());
    DispatchError::Domain {
        kind,
        hint,
        source: Box::new(error),
    }
}

/// Runs each launch step on the runtime actor.
#[derive(Debug)]
struct RuntimeLauncher {
    actor: Actor,
    sessions: LaunchSessions,
    started: Mutex<HashMap<TerminalId, serde_json::Value>>,
}

impl Launcher for RuntimeLauncher {
    fn reserve(&self, launch: Launch<'_>) -> Result<Reservation, DispatchError> {
        let op_id = launch.params.op_id.as_str().to_owned();
        let session = self.sessions.of_op(&op_id)?;
        let (plan, params) = (launch.plan.clone(), launch.params.clone());
        let started = self
            .actor
            .call_blocking(move |runtime| runtime.run_planned(&session, (&plan, &params)))
            .ok_or(DispatchError::Closed)?
            .map_err(domain)?;
        self.sessions.reserved(&op_id, started.terminal);
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(started.terminal, started.result);
        Ok(Reservation::new(
            started.terminal,
            started.process,
            started.waits,
        ))
    }

    fn start(
        &self,
        reservation: &Reservation,
        launch: Launch<'_>,
    ) -> Result<LaunchedTerminal, DispatchError> {
        let result = self
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&reservation.terminal)
            .ok_or(DispatchError::Closed)?;
        launched::from_run(&result, launch.params)
    }

    fn boundary(&self, terminal: TerminalId) -> Result<Boundary, DispatchError> {
        let session = self.sessions.of_terminal(terminal)?;
        let (leases, modes, process) = self
            .actor
            .call_blocking(move |runtime| runtime.planned_boundary(&session, terminal))
            .ok_or(DispatchError::Closed)?
            .map_err(domain)?;
        Ok(Boundary::new(leases, modes, process))
    }

    fn write(&self, terminal: TerminalId, bytes: &[u8]) -> Result<EventSeq, DispatchError> {
        let session = self.sessions.of_terminal(terminal)?;
        let bytes = bytes.to_vec();
        self.actor
            .call_blocking(move |runtime| runtime.write_planned(&session, (terminal, &bytes)))
            .ok_or(DispatchError::Closed)?
            .map_err(domain)
    }
}

/// Registers `template.list`, `template.get`, the template `terminal.run` and
/// `terminal.first_message`.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
) -> Result<(), AssemblyError> {
    let dir = hosting.state.join("templates");
    std::fs::create_dir_all(&dir).map_err(stage("create the template directory"))?;
    let launcher = RuntimeLauncher {
        actor: hosting.actor.clone(),
        sessions: hosting.launches.clone(),
        started: Mutex::default(),
    };
    let lane = TemplateLaunchLane::new(
        (TemplateCatalog::new(dir), launcher),
        Arc::clone(&hosting.authority),
        hosting.identity.machine_id(),
    );
    dispatch
        .register_read(ListTemplates::new(&lane))
        .map_err(stage("register template.list"))?;
    dispatch
        .register_read(GetTemplate::new(&lane))
        .map_err(stage("register template.get"))?;
    dispatch
        .register(RunTerminal::new(&lane))
        .map_err(stage("register the template terminal.run"))?;
    dispatch
        .register(GetFirstMessage::new(&lane))
        .map_err(stage("register terminal.first_message"))
}
