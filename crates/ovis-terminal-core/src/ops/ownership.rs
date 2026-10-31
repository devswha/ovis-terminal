//! Resolve the retained owner before authorizing operation inspection or cancellation.

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::errors::OpId;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{OperationError, OperationEvent, OperationSnapshot, Operations};
use crate::events::{EventData, SnapshotProducer};

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Resolves an operation's retained owner for the authority boundary.
    ///
    /// Operation IDs are principal-scoped: prefer this caller's own record when the
    /// same UUID was independently accepted by several principals. A foreign owner is
    /// returned only for authorization, never as permission to read or mutate its record.
    ///
    /// # Errors
    ///
    /// Returns typed snapshot/store failures. `None` means no retained record names the ID.
    pub fn owner(&self, op: &OpId) -> Result<Option<PrincipalId>, OperationError> {
        let snapshot = self.snapshot()?;
        let mut matching = snapshot
            .rows
            .iter()
            .filter(|row| &row.entry.record.op_id == op);
        let own = matching
            .clone()
            .find(|row| row.entry.principal == self.context.principal);
        Ok(own
            .or_else(|| matching.next())
            .map(|row| row.entry.principal.clone()))
    }
}
