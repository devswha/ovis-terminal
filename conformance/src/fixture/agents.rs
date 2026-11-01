//! The agent status lane over two fixture terminals and their programs' output path.
//!
//! `agent` (terminal:4), `shell` (terminal:5) and `rotation` (terminal:6) each run program
//! incarnation 1 whose spawn token is known to the scenarios; `rotation`'s input goes
//! through the real input sequencer instead of the echo stand-in.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_agent::status::{Ingested, OscOutput, ProcessOwner, StatusService};
use ovis_terminal_authz::{Action, Authority, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{Coordinator, EventError};
use ovis_terminal_core::ops::{Completion, OperationProducer};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity, TerminalId};
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::methods::input::InputSendParams;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_server::dispatch::lanes::agent_status::{
    AgentStatusLane, ClearStatus, GetAgent, GetStatus, RegisterAgent, ReportStatus,
    TerminalDirectory,
};
use ovis_terminal_server::dispatch::{Access, Completed, DispatchError, Execution, Method};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use serde_json::{Value, json};

use super::handler::{Dispatch, Event};
use super::prompts::Gate;
use super::rotation::Sequenced;
use super::serve::at;
use crate::error::RunnerError;

pub mod directory;

pub(crate) use directory::Directory;
use directory::directory;

/// Spawn tokens of the two fixture programs.
const TERMINALS: [(u64, &str, &str); 3] = [
    (4, "agent", "fixture-process-token-agent"),
    (5, "shell", "fixture-process-token-shell"),
    (
        super::rotation::TERMINAL,
        "rotation",
        "fixture-process-token-rotation",
    ),
];

type Core = Coordinator<Event, OperationProducer>;

/// The fixture programs echo their input, as `cat` does, so `input.send` text becomes
/// program output: a real VT engine reads it and every OSC record goes to ingestion.
///
/// This stands in for a PTY until the input path is assembled (todos 28-31); it applies
/// no lease, encoding or submission semantics and returns the echo, not an input receipt.
/// A program blocked on an open prompt or a `blocked` report receives nothing.
#[derive(Debug)]
pub(crate) struct EchoProgram {
    gate: Arc<Gate>,
    service: Arc<StatusService>,
    core: Arc<Core>,
    directory: Arc<Directory>,
    screens: Arc<Mutex<BTreeMap<TerminalId, Vt>>>,
    sequenced: Arc<Sequenced>,
}

/// The status service and terminal directory other agent lanes share.
pub(crate) type AgentParts = (Arc<StatusService>, Arc<Directory>);

/// Registers the agent status methods and the echoing programs, binding their tokens;
/// the returned gate holds the prompts that block those programs' input, and the
/// sequenced terminal is the one whose input `authority` rechecks at its write boundary.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    (runtime, core): (RuntimeIdentity, Arc<Core>),
    authority: &Arc<Authority<RegistryIdentities>>,
) -> Result<(AgentParts, Arc<Gate>, Arc<Sequenced>), RunnerError> {
    let service = Arc::new(StatusService::new(runtime.runtime_id()));
    for (id, _, token) in TERMINALS {
        let terminal = TerminalId::try_from(id).map_err(at("build a terminal id"))?;
        let owner = ProcessOwner::new(terminal, Incarnation::new(1));
        service
            .tokens()
            .bind(&ProcessToken::new(token.to_owned()), owner)
            .map_err(at("bind a process token"))?;
    }
    let directory = Arc::new(directory(runtime)?);
    let lane = AgentStatusLane::new(Arc::clone(&directory), Arc::clone(&service));
    let failed = at("register an agent status method");
    dispatch
        .register(RegisterAgent::new(&lane))
        .map_err(failed)?;
    dispatch
        .register(GetAgent::new(&lane))
        .map_err(at("register agent.get"))?;
    dispatch
        .register(ReportStatus::new(&lane))
        .map_err(at("register status.report"))?;
    dispatch
        .register(GetStatus::new(&lane))
        .map_err(at("register status.get"))?;
    dispatch
        .register(ClearStatus::new(&lane))
        .map_err(at("register status.clear"))?;
    let gate = Gate::new(runtime, &service);
    let sequenced = Sequenced::new(authority, runtime, Arc::clone(&gate.pty))?;
    dispatch
        .register(EchoProgram {
            gate: Arc::clone(&gate),
            service: Arc::clone(&service),
            core,
            directory: Arc::clone(&directory),
            screens: Arc::new(Mutex::new(BTreeMap::new())),
            sequenced: Arc::clone(&sequenced),
        })
        .map_err(at("register input.send"))?;
    Ok(((service, directory), gate, sequenced))
}

fn echo(program: &EchoProgram, terminal: TerminalId, text: &str) -> Result<Value, DispatchError> {
    let internal = |error| DispatchError::Io(std::io::Error::other(error));
    let owner = program
        .service
        .tokens()
        .current(terminal)
        .map_err(internal)?
        .ok_or(DispatchError::Closed)?;
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let now = TimestampMs::new(u64::try_from(millis)?);
    let geometry = Geometry::new(80, 24).map_err(|_| DispatchError::Closed)?;
    let records = program
        .screens
        .lock()
        .map_err(|_| DispatchError::Closed)?
        .entry(terminal)
        .or_insert_with(|| Vt::new(VtConfig::new(geometry, HistoryLimit::new(100))))
        .feed(text.as_bytes())
        .osc;
    let mut ingested = Vec::with_capacity(records.len());
    for record in &records {
        let outcome = program
            .service
            .ingest(&*program.core, OscOutput::new(owner, record, now))
            .map_err(internal)?;
        ingested.push(match outcome {
            Ingested::Applied => "applied",
            Ingested::Dropped => "dropped",
            _ => "ignored",
        });
    }
    program
        .gate
        .pty
        .wrote(terminal, text.len())
        .map_err(|error| DispatchError::Io(std::io::Error::other(error)))?;
    Ok(json!({ "echoed_bytes": text.len().to_string(), "osc_records": ingested }))
}

impl Method<Event, OperationProducer> for EchoProgram {
    const NAME: &'static str = "input.send";
    type Params = InputSendParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let resolved = self.directory.resolve(&params.target)?;
        Ok(Access::new(
            Action::Input,
            ResolvedTarget::new(
                resolved.machine,
                TargetResource::Terminal {
                    placement: None,
                    terminal: resolved.terminal,
                    incarnation: resolved.generation,
                },
            ),
        ))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<Event, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let TargetResource::Terminal { terminal, .. } = execution.target().resource() else {
            return Err(DispatchError::Registration);
        };
        let terminal = *terminal;
        let (gate, core) = (Arc::clone(&self.gate), Arc::clone(&self.core));
        let admitted = tokio::task::spawn_blocking(move || {
            core.read_view(|snapshot| {
                Ok::<_, EventError>(gate.prompts.admission(snapshot, terminal))
            })
        })
        .await?
        .map_err(|error| DispatchError::Io(std::io::Error::other(error)))?;
        if let Err(blocked) = admitted {
            let refusal = blocked.error_data();
            return execution
                .commit_unit(move |_| Ok(Completion::failed(refusal)))
                .await;
        }
        if self.sequenced.serves(terminal) {
            let sequenced = Arc::clone(&self.sequenced);
            let session = Arc::clone(execution.session());
            let op_id = execution.op_id().clone();
            return execution
                .commit_unit(move |_| {
                    Ok(match sequenced.send(&session, (op_id, &params)) {
                        Ok(receipt) => Completion::new(json!({ "receipt": receipt })),
                        Err(refusal) => Completion::failed(*refusal),
                    })
                })
                .await;
        }
        let program = Self {
            gate: Arc::clone(&self.gate),
            service: Arc::clone(&self.service),
            core: Arc::clone(&self.core),
            directory: Arc::clone(&self.directory),
            screens: Arc::clone(&self.screens),
            sequenced: Arc::clone(&self.sequenced),
        };
        execution
            .commit(move || echo(&program, terminal, &params.text))
            .await
    }
}
