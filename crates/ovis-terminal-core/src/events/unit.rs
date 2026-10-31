//! Atomic state, receipt and ordered event units over the existing commit barrier.

use std::sync::atomic::Ordering;

use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{EventKind, Mutation, NewEvent, Payload, Snapshot};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::record::{Body, SCHEMA_VERSION};
use super::{Commit, Coordinator, Cursor, EventData, EventError, SnapshotProducer};

impl<D: EventData + Clone + Serialize + DeserializeOwned, P: SnapshotProducer> Coordinator<D, P> {
    /// Builds one transaction against a stable read view while excluding other commits.
    ///
    /// The closure receives only a read snapshot, never a store handle. All writes,
    /// the receipt and all ordered events commit together. Publication starts only
    /// after the writer acknowledges COMMIT. The returned cursor is the unit's last event.
    ///
    /// # Errors
    ///
    /// Returns the caller's construction error without submitting anything, or a typed
    /// coordinator/store failure. A failed CAS rolls back the complete unit.
    pub fn commit_unit<E>(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Commit<D>, E>,
    ) -> Result<Cursor, E>
    where
        E: From<EventError>,
    {
        let mut state = self.lock().map_err(E::from)?;
        let snapshot = state
            .store
            .snapshot()
            .map_err(EventError::from)
            .map_err(E::from)?;
        let commit = build(&snapshot)?;
        let mut seq = snapshot.revision().get();
        let Commit {
            op,
            data,
            affected,
            writes,
            origin,
            additional,
        } = commit;
        let changes = std::iter::once((data, affected)).chain(additional);
        let mut bodies = Vec::new();
        for (mut data, affected) in changes {
            if affected.is_empty() {
                return Err(E::from(EventError::MissingResources));
            }
            seq = seq
                .checked_add(1)
                .ok_or_else(|| E::from(EventError::LimitExceeded))?;
            data.at_sequence(EventSeq::new(seq));
            let body = Body {
                schema_version: SCHEMA_VERSION,
                identity: self.identity,
                affected,
                origin: origin.clone(),
                data,
            };
            let bytes = serde_json::to_vec(&body)
                .map_err(EventError::from)
                .map_err(E::from)?;
            let charged = bytes.len().saturating_add(1024);
            let event = NewEvent::new(EventKind::new(body.data.event_type()), Payload::from(bytes));
            bodies.push((body.into_event(EventSeq::new(seq)), event, charged));
        }
        let mut records = bodies.iter();
        let (_, first, _) = records
            .next()
            .ok_or_else(|| E::from(EventError::MissingResources))?;
        let mut mutation = Mutation::new(op, Payload::from("{}"), first.clone());
        for (_, event, _) in records {
            mutation = mutation.event(event.clone());
        }
        for write in writes {
            mutation = mutation.compare_and_swap(write);
        }
        drop(snapshot);
        let receipt = state
            .store
            .submit(mutation)
            .and_then(ovis_terminal_store::Ticket::wait)
            .map_err(EventError::from)
            .map_err(E::from)?;
        state.sinks.retain(|sink| sink.lost.strong_count() > 0);
        for (event, _, charged) in bodies {
            for sink in &state.sinks {
                if sink.filter.matches(&event) {
                    match self.charge(charged) {
                        Some(charge) => sink.publish(&event, charge),
                        None => {
                            if let Some(lost) = sink.lost.upgrade() {
                                lost.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                }
            }
        }
        Ok(self.cursor(receipt.revision.get()))
    }
}
