//! Lease transitions made by attach are committed before anyone is told of them.

use std::fmt::Debug;

use ovis_terminal_core::events::{Commit, Coordinator, EventData, ResourceId, SnapshotProducer};
use ovis_terminal_core::leases::{LeaseEvent, Transition};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::OpId;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::StreamError;

/// Durably commits lease transitions as `lease.*` events.
pub trait LeaseJournal: Debug + Send + Sync + 'static {
    /// Commits every event of `transition` in one unit under `op` and returns the
    /// committed sequence, or `None` when the transition changed nothing.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Journal`] when the commit fails; nothing was published.
    fn record(&self, op: OpId, transition: Transition) -> Result<Option<EventSeq>, StreamError>;
}

impl<D, P> LeaseJournal for Coordinator<D, P>
where
    D: EventData
        + From<LeaseEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + Debug
        + 'static,
    P: SnapshotProducer + Send + Sync + Debug + 'static,
{
    fn record(&self, op: OpId, transition: Transition) -> Result<Option<EventSeq>, StreamError> {
        let affected = vec![ResourceId::Terminal(transition.terminal)];
        let mut events = transition.events.into_iter().map(D::from);
        let Some(first) = events.next() else {
            return Ok(None);
        };
        let commit = events.fold(Commit::new(op, first, affected.clone()), |commit, event| {
            commit.event(event, affected.clone())
        });
        Ok(Some(self.commit(commit)?.seq))
    }
}
