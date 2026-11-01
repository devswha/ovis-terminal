//! Assembly of the agent domain over one dispatcher and its one commit coordinator
//! (plan todo 42).
//!
//! Status, prompts, conversation records and session binding share one
//! [`StatusService`], so every report, prompt and record is bound to the same
//! registered program and committed in the dispatcher's single sequence domain. The
//! prompt bridge gate wraps the served handler so a program token can only open or
//! close prompts on its own terminal.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::{BindingService, Stores};
use ovis_terminal_agent::conversation::records::RecordsService;
use ovis_terminal_agent::prompts::{KeyInput, PromptService};
use ovis_terminal_agent::status::{AgentStatusEvent, StatusService};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{MachineId, RuntimeIdentity};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::lanes::agent_status::{
    AgentStatusLane, ClearStatus, GetAgent, GetStatus, RegisterAgent, ReportStatus,
    TerminalDirectory,
};
use super::lanes::conversation_records::{AppendRecords, ConversationRecordsLane, Records};
use super::lanes::prompt_bridge::{Gated, PromptBridgeGate};
use super::lanes::prompts::{
    AnswerPrompt, ClosePrompt, GetPrompt, ListPrompts, OpenPrompt, PromptLane,
};
use super::lanes::session_binding::{Conversation, SessionBindingLane, UpdateSession};
use super::{DispatchError, Dispatcher};

/// Every method [`AgentAssembly::register`] routes, in registration order.
pub const AGENT_METHODS: [&str; 14] = [
    "agent.register",
    "agent.get",
    "status.report",
    "status.get",
    "status.clear",
    "prompt.open",
    "prompt.answer",
    "prompt.close",
    "prompt.get",
    "prompt.list",
    "agent.append_records",
    "agent.records",
    "agent.update_session",
    "agent.conversation",
];

/// The agent services one runtime serves, bound to one terminal directory.
#[derive(Debug)]
pub struct AgentAssembly<T> {
    terminals: T,
    machine: MachineId,
    status: Arc<StatusService>,
    prompts: Arc<PromptService>,
    records: Arc<RecordsService>,
    binding: Arc<BindingService>,
}

impl<T: TerminalDirectory + Clone> AgentAssembly<T> {
    /// Builds the services of `runtime` over `terminals`, reading conversations only
    /// from the authorized `stores`; typed prompt answers reach programs through `keys`.
    pub fn new(
        runtime: RuntimeIdentity,
        terminals: T,
        (stores, keys): (Stores, Option<Arc<dyn KeyInput>>),
    ) -> Self {
        let id = runtime.runtime_id();
        let status = Arc::new(StatusService::new(id));
        let prompts = PromptService::new(id, Arc::clone(&status));
        let prompts = Arc::new(match keys {
            Some(keys) => prompts.with_keys(keys),
            None => prompts,
        });
        Self {
            terminals,
            machine: runtime.machine_id(),
            records: Arc::new(RecordsService::new(id, Arc::clone(&status))),
            binding: Arc::new(BindingService::new(id, Arc::clone(&status), stores)),
            status,
            prompts,
        }
    }

    /// The registered programs, their tokens and declared status.
    pub const fn status(&self) -> &Arc<StatusService> {
        &self.status
    }

    /// The versioned prompts that gate ordinary input.
    pub const fn prompts(&self) -> &Arc<PromptService> {
        &self.prompts
    }

    /// Serves `inner` behind the prompt bridge's program-token gate.
    pub fn gate<H>(&self, inner: Arc<H>) -> Gated<H, T> {
        let services = (Arc::clone(&self.status), Arc::clone(&self.prompts));
        Gated::new(
            PromptBridgeGate::new(self.terminals.clone(), services),
            inner,
        )
    }

    /// Routes every [`AGENT_METHODS`] entry; reads are unrecorded.
    ///
    /// # Errors
    ///
    /// Returns `registration` when a method is already routed.
    pub fn register<D, P>(&self, dispatcher: &mut Dispatcher<D, P>) -> Result<(), DispatchError>
    where
        D: EventData + From<OperationEvent> + From<AgentStatusEvent> + Clone + Serialize,
        D: DeserializeOwned + Send + 'static,
        P: SnapshotProducer + Send + Sync + 'static,
        P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
    {
        let status = AgentStatusLane::new(self.terminals.clone(), Arc::clone(&self.status));
        dispatcher.register(RegisterAgent::new(&status))?;
        dispatcher.register(GetAgent::new(&status))?;
        dispatcher.register(ReportStatus::new(&status))?;
        dispatcher.register(GetStatus::new(&status))?;
        dispatcher.register(ClearStatus::new(&status))?;
        let prompts = PromptLane::new(
            self.terminals.clone(),
            Arc::clone(&self.prompts),
            self.machine,
        );
        dispatcher.register(OpenPrompt::new(&prompts))?;
        dispatcher.register(AnswerPrompt::new(&prompts))?;
        dispatcher.register(ClosePrompt::new(&prompts))?;
        dispatcher.register_read(GetPrompt::new(&prompts))?;
        dispatcher.register_read(ListPrompts::new(&prompts))?;
        let records =
            ConversationRecordsLane::new(self.terminals.clone(), Arc::clone(&self.records));
        dispatcher.register(AppendRecords::new(&records))?;
        dispatcher.register_read(Records::new(&records))?;
        let binding = SessionBindingLane::new(self.terminals.clone(), Arc::clone(&self.binding));
        dispatcher.register(UpdateSession::new(&binding))?;
        dispatcher.register(Conversation::new(&binding))
    }
}
