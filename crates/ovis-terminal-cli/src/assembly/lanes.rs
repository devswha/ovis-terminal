//! The landed dispatcher lanes the server hosts behind its own routes: documents, plugins,
//! machine identity, agent status, prompts and session binding, on one dispatcher whose
//! coordinator commits their operation, agent and document events.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_agent::conversation::binding::{BindingService, Stores};
use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::{AgentStatusEvent, StatusService};
use ovis_terminal_authz::Authority;
use ovis_terminal_core::docs::DocChanged;
use ovis_terminal_core::events::{Coordinator, EventData};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_server::dispatch::lanes::agent_status::{
    AgentStatusLane, ClearStatus, GetAgent, GetStatus, RegisterAgent, ReportStatus,
};
use ovis_terminal_server::dispatch::lanes::machine_identity::{
    MachineGet, MachineLane, MachineRename,
};
use ovis_terminal_server::dispatch::lanes::prompts::{
    AnswerPrompt, ClosePrompt, GetPrompt, ListPrompts, OpenPrompt, PromptLane,
};
use ovis_terminal_server::dispatch::lanes::session_binding::{
    Conversation, SessionBindingLane, UpdateSession,
};
use ovis_terminal_server::dispatch::lanes::{documents_retention, plugins};
use ovis_terminal_services::plugins::Plugins;
use ovis_terminal_store::{Store, StoreOptions};
use serde::{Deserialize, Serialize};

use super::AssemblyError;
use super::directory::Directory;

/// Bound on one remote machine handshake.
const HANDSHAKE_BOUND: Duration = Duration::from_secs(10);

/// Every event the hosted lanes commit, in one sequence domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "lane", content = "event", rename_all = "snake_case")]
pub(super) enum ServedEvent {
    /// Operation, agent status, session and prompt events.
    Agent(AgentStatusEvent),
    /// A document changed.
    Doc(DocChanged),
}

impl EventData for ServedEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Agent(event) => event.event_type(),
            Self::Doc(event) => event.event_type(),
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Agent(event) => event.at_sequence(seq),
            Self::Doc(event) => event.at_sequence(seq),
        }
    }
}

impl From<OperationEvent> for ServedEvent {
    fn from(event: OperationEvent) -> Self {
        Self::Agent(AgentStatusEvent::from(event))
    }
}

impl From<AgentStatusEvent> for ServedEvent {
    fn from(event: AgentStatusEvent) -> Self {
        Self::Agent(event)
    }
}

impl From<DocChanged> for ServedEvent {
    fn from(event: DocChanged) -> Self {
        Self::Doc(event)
    }
}

/// The dispatcher of the hosted lanes.
pub(super) type Dispatch = Dispatcher<ServedEvent, OperationProducer>;
/// Its coordinator, closed after the dispatcher shut down.
pub(super) type DispatchLog = Coordinator<ServedEvent, OperationProducer>;

/// What the hosted lanes need from the server lifetime.
pub(super) struct Hosting<'a> {
    pub(super) state: &'a Path,
    pub(super) identity: RuntimeIdentity,
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) directory: Arc<Directory>,
}

fn stage<E: Into<super::StageSource>>(stage: &'static str) -> impl FnOnce(E) -> AssemblyError {
    AssemblyError::start(stage)
}

/// Builds the dispatcher with every hosted lane registered and prompt routes recovered.
pub(super) fn host(hosting: Hosting<'_>) -> Result<(Dispatch, Arc<DispatchLog>), AssemblyError> {
    let Hosting {
        state,
        identity,
        authority,
        directory,
    } = hosting;
    let store = Store::open(&state.join("dispatch.db"), StoreOptions::default())
        .map_err(stage("open the dispatch store"))?;
    let log = Arc::new(Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    ));
    let mut dispatch = Dispatcher::new(authority, identity, Arc::clone(&log));
    let machine = identity.machine_id();
    documents_retention::register(&mut dispatch, identity).map_err(stage("register documents"))?;
    std::fs::create_dir_all(state.join("plugins")).map_err(stage("create the plugin root"))?;
    let (list, invoke, run) =
        plugins::routes(&Arc::new(Plugins::new(state.join("plugins"))), machine);
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
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    let agents = AgentStatusLane::new(Arc::clone(&directory), Arc::clone(&status));
    dispatch
        .register(RegisterAgent::new(&agents))
        .map_err(stage("register agent.register"))?;
    dispatch
        .register(GetAgent::new(&agents))
        .map_err(stage("register agent.get"))?;
    dispatch
        .register(ReportStatus::new(&agents))
        .map_err(stage("register status.report"))?;
    dispatch
        .register(GetStatus::new(&agents))
        .map_err(stage("register status.get"))?;
    dispatch
        .register(ClearStatus::new(&agents))
        .map_err(stage("register status.clear"))?;
    let prompts = Arc::new(PromptService::new(
        identity.runtime_id(),
        Arc::clone(&status),
    ));
    log.read_view(|snapshot| prompts.recover(snapshot))
        .map_err(stage("recover open prompts"))?;
    let lane = PromptLane::new(Arc::clone(&directory), prompts, machine);
    dispatch
        .register(OpenPrompt::new(&lane))
        .map_err(stage("register prompt.open"))?;
    dispatch
        .register(AnswerPrompt::new(&lane))
        .map_err(stage("register prompt.answer"))?;
    dispatch
        .register(ClosePrompt::new(&lane))
        .map_err(stage("register prompt.close"))?;
    dispatch
        .register_read(GetPrompt::new(&lane))
        .map_err(stage("register prompt.get"))?;
    dispatch
        .register_read(ListPrompts::new(&lane))
        .map_err(stage("register prompt.list"))?;
    let stores = Stores::new(&[]).map_err(stage("authorize session stores"))?;
    let binding = Arc::new(BindingService::new(identity.runtime_id(), status, stores));
    let sessions = SessionBindingLane::new(directory, binding);
    dispatch
        .register(UpdateSession::new(&sessions))
        .map_err(stage("register agent.update_session"))?;
    dispatch
        .register(Conversation::new(&sessions))
        .map_err(stage("register agent.conversation"))?;
    Ok((dispatch, log))
}
