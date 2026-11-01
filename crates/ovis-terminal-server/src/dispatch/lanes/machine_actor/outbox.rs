//! The agent tree's durable outbox as the orphan outbox, and its receiving-side apply.

use std::sync::Arc;

use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_agent::tree::{Exit, TreeError, TreeService};
use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_protocol::domain::agent_tree::{OrphanAck, OrphanNotice};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_remote::orphans::{Outbox, OutboxError};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn failed(error: TreeError) -> OutboxError {
    OutboxError::new((error.kind(), error.hint()), error)
}

fn joined(error: tokio::task::JoinError) -> OutboxError {
    OutboxError::new(("internal", "retry; the store task did not finish"), error)
}

/// Orphan notices kept in the agent tree's store until the receiving machine acknowledges.
pub struct TreeOutbox<D, P> {
    tree: Arc<TreeService>,
    core: Arc<Coordinator<D, P>>,
}

impl<D, P> std::fmt::Debug for TreeOutbox<D, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TreeOutbox")
    }
}

impl<D, P> TreeOutbox<D, P>
where
    D: EventData
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    /// The outbox of `tree`, committed through `core`.
    #[must_use]
    pub const fn new(tree: Arc<TreeService>, core: Arc<Coordinator<D, P>>) -> Self {
        Self { tree, core }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&TreeService, &Coordinator<D, P>) -> Result<T, TreeError> + Send + 'static,
    ) -> Result<T, OutboxError> {
        let (tree, core) = (Arc::clone(&self.tree), Arc::clone(&self.core));
        tokio::task::spawn_blocking(move || work(&tree, &core))
            .await
            .map_err(joined)?
            .map_err(failed)
    }

    /// Applies a parent's death received from its machine; a replay ends nothing.
    ///
    /// # Errors
    ///
    /// The agent tree's store failure.
    pub async fn apply(&self, notice: OrphanNotice) -> Result<Exit, OutboxError> {
        self.blocking(move |tree, core| tree.orphaned(core, &notice))
            .await
    }
}

impl<D, P> Outbox for TreeOutbox<D, P>
where
    D: EventData
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    async fn pending(&self, machine: &MachineId) -> Result<Vec<OrphanNotice>, OutboxError> {
        let machine = machine.clone();
        self.blocking(move |tree, core| core.read_view(|snapshot| tree.outbox(snapshot, &machine)))
            .await
    }

    async fn acknowledge(
        &self,
        machine: &MachineId,
        notice: &OrphanNotice,
    ) -> Result<bool, OutboxError> {
        let ack = OrphanAck::new(notice.clone(), machine.clone());
        self.blocking(move |tree, core| tree.orphan_acked(core, &ack))
            .await
    }
}
