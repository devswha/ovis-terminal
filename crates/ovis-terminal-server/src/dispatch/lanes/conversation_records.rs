//! `agent.append_records` and `agent.records` (plan todo 37).
//!
//! Both resolve their explicit target through the owning terminal directory. An append
//! runs inside one commit unit, committing its record entries, head and
//! `agent.records_appended` event together; its receipt names the records by identity,
//! count and digest. A read is answered from one committed snapshot outside the
//! operation ledger, so pages of any size never meet the receipt bound.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::Read as RecordsRead;
use ovis_terminal_agent::conversation::records::{Append, RecordsError, RecordsService};
use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_authz::{Action, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::domain::conversation_records::{
    AgentRecordsParams, AppendRecordsParams,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::agent_status::{TerminalDirectory, terminal, terminal_access};
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method, Read};

/// Shared state of the conversation record methods.
#[derive(Debug)]
pub struct ConversationRecordsLane<T> {
    terminals: T,
    records: Arc<RecordsService>,
}

impl<T: TerminalDirectory> ConversationRecordsLane<T> {
    /// Serves `records` for the terminals `terminals` resolves.
    pub fn new(terminals: T, records: Arc<RecordsService>) -> Arc<Self> {
        Arc::new(Self { terminals, records })
    }
}

/// `agent.append_records`: typed records reported by the owning process.
#[derive(Debug)]
pub struct AppendRecords<T>(Arc<ConversationRecordsLane<T>>);

/// `agent.records`: one page of the reported records, read outside the ledger.
#[derive(Debug)]
pub struct Records<T>(Arc<ConversationRecordsLane<T>>);

impl<T> AppendRecords<T> {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<ConversationRecordsLane<T>>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<T> Records<T> {
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

impl<T: TerminalDirectory> Read for Records<T> {
    const NAME: &'static str = "agent.records";
    type Params = AgentRecordsParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        terminal_access(&self.0.terminals, &params.target, Action::Read)
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let terminal = self
            .0
            .terminals
            .resolve(&params.target)
            .map_err(|error| DispatchError::from(error).boundary())?
            .terminal;
        let mut read = RecordsRead::new(terminal);
        read.cursor = params.cursor;
        read.page = params.page;
        let refused = |error: RecordsError| BoundaryError::Domain {
            data: Box::new(error.error_data()),
        };
        let page = self.0.records.read(snapshot, &read).map_err(refused)?;
        serde_json::to_value(page).map_err(|error| refused(RecordsError::Encode(error)))
    }
}
