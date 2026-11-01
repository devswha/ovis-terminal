//! Consumer-side transitions of one entry: drain, close, filter replacement and restart.

use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::table::{Lost, Phase, Table};
use super::{Batch, Closed, Selector, SubscriptionError};

/// What a consumer finds when it looks at its own entry.
#[derive(Debug)]
pub(super) enum Step<D> {
    Events(Batch<D>),
    Lost,
    Closed(Closed),
    Idle(Closed),
}

impl<D> Table<D> {
    /// Hands every queued event to the consumer, or reports why there is none.
    pub(super) fn take(&mut self, id: &SubscriptionId) -> Result<Step<D>, SubscriptionError> {
        let index = self.index(id)?;
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(SubscriptionError::NotFound)?;
        let position = Closed::at(entry.delivered, entry.scan);
        let step = match entry.phase {
            Phase::Closed => Step::Closed(position),
            Phase::Lost(_) => Step::Lost,
            Phase::Active if entry.queue.is_empty() => Step::Idle(position),
            Phase::Active => {
                let events: Vec<_> = entry.queue.drain(..).map(|(event, _)| event).collect();
                let released = std::mem::take(&mut entry.bytes);
                entry.report_taken(released);
                if let Some(last) = events.last() {
                    entry.delivered = last.cursor.seq.get();
                }
                self.session_bytes = self.session_bytes.saturating_sub(released);
                Step::Events(Batch {
                    events,
                    scan_seq: EventSeq::new(position.scan_seq.get()),
                })
            }
        };
        Ok(step)
    }

    /// Replaces the selector; the returned sequence is the first one judged by it.
    pub(super) fn update(
        &mut self,
        id: &SubscriptionId,
        selector: Selector,
    ) -> Result<EventSeq, SubscriptionError> {
        let entry = self.find(id)?;
        let effective = entry
            .scan
            .checked_add(1)
            .ok_or(SubscriptionError::SequenceExhausted)?;
        entry.selector = selector;
        Ok(EventSeq::new(effective))
    }

    /// Closes the entry and releases its queue; its consumer reads the position once.
    pub(super) fn close(&mut self, id: &SubscriptionId) -> Result<Closed, SubscriptionError> {
        let entry = self.find(id)?;
        entry.queue.clear();
        let released = std::mem::take(&mut entry.bytes);
        entry.report_dropped(released);
        entry.gauge = None;
        entry.phase = Phase::Closed;
        let closed = Closed::at(entry.delivered, entry.scan);
        self.session_bytes = self.session_bytes.saturating_sub(released);
        Ok(closed)
    }

    /// Whether the entry lost delivery and its marker is already durable.
    pub(super) fn awaiting_restart(&self, id: &SubscriptionId) -> Result<bool, SubscriptionError> {
        let entry = self
            .entries
            .get(self.index(id)?)
            .ok_or(SubscriptionError::NotFound)?;
        Ok(matches!(entry.phase, Phase::Lost(lost) if lost.persisted))
    }

    /// Restarts a lost entry whose marker is durable at a fresh snapshot sequence.
    pub(super) fn restart(
        &mut self,
        id: &SubscriptionId,
        seq: u64,
    ) -> Result<Option<Lost>, SubscriptionError> {
        let index = self.index(id)?;
        let Some(entry) = self.entries.get_mut(index) else {
            return Err(SubscriptionError::NotFound);
        };
        let Phase::Lost(lost) = entry.phase else {
            return Ok(None);
        };
        if !lost.persisted {
            return Ok(None);
        }
        entry.scan = seq;
        entry.delivered = seq;
        entry.phase = Phase::Active;
        entry.report_resynced(lost);
        Ok(Some(lost))
    }
}
