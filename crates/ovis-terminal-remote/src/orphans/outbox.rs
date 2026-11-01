//! The durable store of orphan notices not yet acknowledged.

use std::future::Future;

use ovis_terminal_protocol::domain::agent_tree::OrphanNotice;
use ovis_terminal_protocol::schema::ids::MachineId;

use super::OutboxError;

/// Notices kept per receiving machine until it acknowledges them; survives restarts.
pub trait Outbox: Send + Sync {
    /// The notices `machine` has not acknowledged, oldest first.
    ///
    /// # Errors
    ///
    /// The store failure.
    fn pending(
        &self,
        machine: &MachineId,
    ) -> impl Future<Output = Result<Vec<OrphanNotice>, OutboxError>> + Send;

    /// Durably removes `notice` from `machine`'s outbox; returns whether it was pending.
    ///
    /// # Errors
    ///
    /// The store failure; the notice then stays pending.
    fn acknowledge(
        &self,
        machine: &MachineId,
        notice: &OrphanNotice,
    ) -> impl Future<Output = Result<bool, OutboxError>> + Send;
}
