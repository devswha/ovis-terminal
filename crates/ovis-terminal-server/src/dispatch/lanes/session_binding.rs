//! `agent.update_session` (plan todo 36).
//!
//! The method resolves its explicit target through the owning terminal directory and
//! runs inside one commit unit: a binding change commits its history record, agent row
//! and `agent.session_bound` event together. Its history is read through
//! `agent.conversation` in the conversation records lane.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::{BindingService, Update};
use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_authz::{Action, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::schema::methods::agent::AgentUpdateSessionParams;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::agent_status::{TerminalDirectory, terminal_access};
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// Shared state of the session binding methods.
#[derive(Debug)]
pub struct SessionBindingLane<T> {
    terminals: T,
    service: Arc<BindingService>,
}

impl<T: TerminalDirectory> SessionBindingLane<T> {
    /// Serves `service` for the terminals `terminals` resolves.
    pub fn new(terminals: T, service: Arc<BindingService>) -> Arc<Self> {
        Arc::new(Self { terminals, service })
    }
}

/// `agent.update_session`: an explicit binding change by the owning process.
#[derive(Debug)]
pub struct UpdateSession<T>(Arc<SessionBindingLane<T>>);

impl<T> UpdateSession<T> {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<SessionBindingLane<T>>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<T, D, P> Method<D, P> for UpdateSession<T>
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
    const NAME: &'static str = "agent.update_session";
    type Params = AgentUpdateSessionParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        terminal_access(&self.0.terminals, &params.target, Action::Report)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let terminal = super::agent_status::terminal(&execution)?;
        let mut update = Update::new(terminal, params.process_token, params.session);
        update.history_id = params.history_id;
        update.reason = params.reason;
        update.head = params.head;
        let service = Arc::clone(&self.0.service);
        execution
            .commit_unit(move |snapshot: &_| service.update_session(snapshot, update))
            .await
    }
}
