//! A settled operation's durable record, read without waiting for a snapshot slot.

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState};
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum};
use ovis_terminal_store::ResourceLookup;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::super::ledger::{Entry, OperationKey};
use super::Operations;
use crate::events::{EventData, EventError, SnapshotProducer};
use crate::ops::{OperationError, OperationEvent, OperationSnapshot};

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// The committed, failed, cancelled or uncertain record of `op`, if it has one.
    ///
    /// A retry of a settled identity waits for admission like new work, yet the ledger
    /// already holds that identity's outcome; an inspection must report the durable
    /// outcome, never the retry's queue place. The read opens one WAL view of the row
    /// and never takes a pinned-snapshot slot, so it cannot queue behind the retry.
    ///
    /// # Errors
    ///
    /// Returns typed store or decoding failures; a missing or running row is `None`.
    pub fn settled(&self, op: &OpId) -> Result<Option<OpRecord>, OperationError> {
        let key = OperationKey::new(&self.context.principal, op)?;
        let resource = key.resource(self.context.identity.runtime_id());
        let entry = self.coordinator.read_view(|view| {
            match view.resource(&resource).map_err(EventError::from)? {
                ResourceLookup::Live {
                    value, revision, ..
                } => {
                    let mut entry: Entry = serde_json::from_slice(value.as_bytes())?;
                    entry.record.updated_seq = EventSeq::new(revision.get());
                    Ok::<_, OperationError>(Some(entry))
                }
                _ => Ok(None),
            }
        })?;
        Ok(entry
            .filter(|entry| self.retained(entry).is_ok() && is_settled(&entry.record))
            .map(|entry| entry.record))
    }
}

const fn is_settled(record: &OpRecord) -> bool {
    matches!(
        record.state,
        OpenEnum::Known(
            OpState::Committed | OpState::Failed | OpState::Cancelled | OpState::OutcomeUnknown
        )
    )
}
