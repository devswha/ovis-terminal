//! `agent.append_records` and `agent.conversation` (plan todo 37).
//!
//! Both resolve their explicit target through the owning terminal directory and run
//! inside one commit unit. An append commits the record log and its
//! `agent.records_appended` event together; a read serves the bound transcript, or with
//! `source: records` the typed records the owning process reported. No source reads
//! terminal output; any other `source` is refused while parameters decode.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::{BindingService, Read};
use ovis_terminal_agent::conversation::records::{Append, RecordsService};
use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_authz::{Action, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::domain::conversation_records::{
    AppendRecordsParams, ConversationSource,
};
use ovis_terminal_protocol::schema::methods::agent::AgentConversationParams;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::agent_status::{TerminalDirectory, terminal, terminal_access};
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// Shared state of the conversation record methods.
#[derive(Debug)]
pub struct ConversationRecordsLane<T> {
    terminals: T,
    binding: Arc<BindingService>,
    records: Arc<RecordsService>,
}

impl<T: TerminalDirectory> ConversationRecordsLane<T> {
    /// Serves transcripts through `binding` and reported records through `records`.
    pub fn new(
        terminals: T,
        binding: Arc<BindingService>,
        records: Arc<RecordsService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            terminals,
            binding,
            records,
        })
    }
}

/// `agent.append_records`: typed records reported by the owning process.
#[derive(Debug)]
pub struct AppendRecords<T>(Arc<ConversationRecordsLane<T>>);

/// `agent.conversation`: one page of the bound transcript or of the reported records.
#[derive(Debug)]
pub struct Conversation<T>(Arc<ConversationRecordsLane<T>>);

impl<T> AppendRecords<T> {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<ConversationRecordsLane<T>>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<T> Conversation<T> {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<ConversationRecordsLane<T>>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<T, D, P> Method<D, P> for AppendRecords<T>
where
    T: TerminalDirectory,
    D: EventData
        + From<OperationEvent>
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "agent.append_records";
    type Params = AppendRecordsParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        terminal_access(&self.0.terminals, &params.target, Action::Report)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let mut append = Append::new(terminal(&execution)?, params.process_token, params.records);
        append.turn = params.turn;
        let records = Arc::clone(&self.0.records);
        execution
            .commit_unit(move |snapshot: &_| records.append(snapshot, append))
            .await
    }
}

impl<T, D, P> Method<D, P> for Conversation<T>
where
    T: TerminalDirectory,
    D: EventData
        + From<OperationEvent>
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "agent.conversation";
    type Params = AgentConversationParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        terminal_access(&self.0.terminals, &params.target, Action::Read)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let mut read = Read::new(terminal(&execution)?);
        read.cursor = params.cursor;
        read.page = params.page;
        if params.source == Some(ConversationSource::Records) {
            let records = Arc::clone(&self.0.records);
            return execution
                .commit_unit(move |snapshot: &_| records.conversation(snapshot, &read))
                .await;
        }
        let binding = Arc::clone(&self.0.binding);
        execution
            .commit_unit(move |snapshot: &_| binding.conversation(snapshot, &read))
            .await
    }
}
