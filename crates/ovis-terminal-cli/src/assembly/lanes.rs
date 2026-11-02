//! The landed dispatcher lanes the server hosts behind its own routes: health, documents,
//! plugins, machine identity, agent status, prompts and session binding, on one dispatcher whose
//! coordinator commits their operation, agent and document events. Each registration
//! phase lives in its own child module, so a later lane adds one call here.

pub(super) mod agents;
mod machine;
pub(super) mod remote;
mod service;
pub(super) mod templates;
mod update;
mod worktrees;

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_authz::Authority;
use ovis_terminal_core::docs::DocChanged;
use ovis_terminal_core::events::{Coordinator, EventData};
use ovis_terminal_core::leases::LeaseEvent;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_core::subscriptions::LossRecord;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::dispatch::Dispatcher;
use ovis_terminal_services::worktrees::WorktreeEvent;
use ovis_terminal_store::{Store, StoreOptions};
use serde::{Deserialize, Serialize};

use super::AssemblyError;
use super::directory::Directory;

/// Every event the hosted lanes commit, in one sequence domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "lane", content = "event", rename_all = "snake_case")]
pub(super) enum ServedEvent {
    /// Operation, agent status, session and prompt events.
    Agent(AgentStatusEvent),
    /// A document changed.
    Doc(DocChanged),
    /// A checkout was created, opened or removed.
    Worktree(WorktreeEvent),
    /// A subscription lost matching events.
    Lost(LossRecord),
    /// An attach lease was granted, moved or revoked (published from the lease journal).
    Lease(LeaseEvent),
}

impl ServedEvent {
    /// The event's own data, as an `event` notification carries it.
    pub(super) fn payload(&self) -> Result<serde_json::Value, serde_json::Error> {
        match self {
            Self::Agent(event) => serde_json::to_value(event),
            Self::Doc(event) => serde_json::to_value(event),
            Self::Worktree(event) => serde_json::to_value(event),
            Self::Lost(loss) => serde_json::to_value(loss),
            Self::Lease(event) => super::lease_feed::payload(event),
        }
    }
}

impl EventData for ServedEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Agent(event) => event.event_type(),
            Self::Doc(event) => event.event_type(),
            Self::Worktree(event) => event.event_type(),
            Self::Lost(_) => "subscription.lost",
            Self::Lease(event) => event.event_type(),
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Agent(event) => event.at_sequence(seq),
            Self::Doc(event) => event.at_sequence(seq),
            Self::Worktree(event) => event.at_sequence(seq),
            Self::Lease(event) => event.at_sequence(seq),
            Self::Lost(_) => {}
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

impl From<WorktreeEvent> for ServedEvent {
    fn from(event: WorktreeEvent) -> Self {
        Self::Worktree(event)
    }
}

impl From<LeaseEvent> for ServedEvent {
    fn from(event: LeaseEvent) -> Self {
        Self::Lease(event)
    }
}

impl From<LossRecord> for ServedEvent {
    fn from(loss: LossRecord) -> Self {
        Self::Lost(loss)
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
    pub(super) actor: super::actor::Actor,
    pub(super) control: Arc<ovis_terminal_core::restart::Control>,
    pub(super) agents: Arc<agents::Agents>,
    pub(super) health: Arc<ovis_terminal_core::health::Health>,
    pub(super) launches: templates::LaunchSessions,
}

fn stage<E: Into<super::StageSource>>(stage: &'static str) -> impl FnOnce(E) -> AssemblyError {
    AssemblyError::start(stage)
}

/// Builds the dispatcher with every hosted lane registered and prompt routes recovered.
pub(super) fn host(
    hosting: &Hosting<'_>,
) -> Result<(Dispatch, Arc<DispatchLog>, remote::Routes), AssemblyError> {
    let store = Store::open(&hosting.state.join("dispatch.db"), StoreOptions::default())
        .map_err(stage("open the dispatch store"))?;
    let identity = hosting.identity;
    let log = Arc::new(Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    ));
    let mut dispatch = Dispatcher::new(Arc::clone(&hosting.authority), identity, Arc::clone(&log));
    ovis_terminal_server::dispatch::lanes::health::register(
        &mut dispatch,
        &hosting.health,
        identity,
    )
    .map_err(stage("register server.status and server.metrics"))?;
    machine::register(&mut dispatch, hosting)?;
    agents::register(&mut dispatch, hosting, &log)?;
    templates::register(&mut dispatch, hosting)?;
    update::register(&mut dispatch, hosting)?;
    worktrees::register(&mut dispatch, hosting, &log)?;
    let routes = remote::register(&mut dispatch, hosting, &log)?;
    Ok((dispatch, log, routes))
}
