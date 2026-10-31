//! Rebuild event envelopes from the same durable rows used for live delivery.

use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{EventPage, Revision, Snapshot};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::record::{Body, SCHEMA_VERSION};
use super::{Coordinator, Cursor, Event, EventData, EventError, MissedReason, SnapshotProducer};

pub(super) struct Replay<D> {
    pub events: Vec<Event<D>>,
    pub missed: Option<MissedReason>,
}

impl<D: EventData + Clone + Serialize + DeserializeOwned, P: SnapshotProducer> Coordinator<D, P> {
    /// Lowest cursor sequence a resume can still replay from, read from the durable floor.
    ///
    /// # Errors
    ///
    /// Returns `coordinator_stopped` after poisoning or a typed store read failure.
    pub fn replay_floor(&self) -> Result<EventSeq, EventError> {
        let view = self.lock()?.store.snapshot()?;
        match view.events_after(Revision::ZERO, 1)? {
            EventPage::Expired { floor } => Ok(EventSeq::new(floor.get().saturating_sub(1))),
            EventPage::Events(_) => Ok(EventSeq::new(0)),
            _ => Err(EventError::InvalidRecord),
        }
    }

    pub(super) fn replay(
        &self,
        snapshot: &Snapshot,
        cursor: Cursor,
    ) -> Result<Replay<D>, EventError> {
        if cursor.machine_id != self.identity.machine_id()
            || cursor.runtime_id != self.identity.runtime_id()
        {
            return Ok(Replay {
                events: Vec::new(),
                missed: Some(MissedReason::LifetimeMismatch),
            });
        }
        if cursor.seq.get() > snapshot.revision().get() {
            return Err(EventError::SequenceAhead);
        }
        match snapshot.events_after(
            Revision::new(cursor.seq.get()),
            self.limits.queued_events.saturating_add(1),
        )? {
            EventPage::Expired { .. } => Ok(Replay {
                events: Vec::new(),
                missed: Some(MissedReason::CursorExpired),
            }),
            EventPage::Events(events) => {
                if events.len() > self.limits.queued_events {
                    return Err(EventError::LimitExceeded);
                }
                let mut decoded = Vec::with_capacity(events.len());
                for event in events {
                    let body: Body<D> = serde_json::from_slice(event.data.as_bytes())
                        .map_err(EventError::DecodeRecord)?;
                    if body.schema_version != SCHEMA_VERSION
                        || body.identity.machine_id() != self.identity.machine_id()
                        || body.identity.runtime_id() != self.identity.runtime_id()
                        || body.data.event_type() != event.kind.as_str()
                    {
                        return Err(EventError::InvalidRecord);
                    }
                    decoded.push(body.into_event(EventSeq::new(event.seq.get())));
                }
                Ok(Replay {
                    events: decoded,
                    missed: None,
                })
            }
            _ => Err(EventError::InvalidRecord),
        }
    }
}
