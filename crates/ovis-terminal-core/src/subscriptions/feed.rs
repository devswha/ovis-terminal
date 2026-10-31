//! One consumer's pull side: batches, idle heartbeats, loss recovery and close.

use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};
use std::time::Instant;

use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::consume::Step;
use super::shared::Shared;
use super::table::Table;
use super::{LossRecord, SubscriptionError};
use crate::events::{Event, EventData, SnapshotProducer, StateSnapshot};
use crate::registry::{MachineId, RuntimeId, ServerBootId};

/// Matching events in commit order plus the highest sequence this subscription examined.
#[derive(Debug)]
#[non_exhaustive]
pub struct Batch<D> {
    /// Matching events, strictly after every earlier batch.
    pub events: Vec<Event<D>>,
    /// Highest sequence examined; gaps up to it were nonmatching, not lost.
    pub scan_seq: EventSeq,
}

/// Idle liveness report of one subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Heartbeat {
    /// Subscription.
    pub subscription_id: SubscriptionId,
    /// Highest sequence delivered.
    pub seq: EventSeq,
    /// Highest sequence examined, including nonmatching events.
    pub scan_seq: EventSeq,
    /// Machine committing the sequence.
    pub machine_id: MachineId,
    /// Runtime lifetime the sequence belongs to.
    pub runtime_id: RuntimeId,
    /// Control-server boot serving the subscription.
    pub boot_id: ServerBootId,
}

/// Where a closed subscription stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Closed {
    /// Highest sequence delivered.
    pub seq: EventSeq,
    /// Highest sequence examined.
    pub scan_seq: EventSeq,
}

impl Closed {
    pub(super) const fn at(delivered: u64, scan: u64) -> Self {
        Self {
            seq: EventSeq::new(delivered),
            scan_seq: EventSeq::new(scan),
        }
    }
}

/// A durable loss marker followed by the fresh snapshot delivery resumes after.
#[derive(Debug)]
#[non_exhaustive]
pub struct Recovery<S> {
    /// The committed loss marker.
    pub loss: LossRecord,
    /// Complete state; later batches hold only events after its cursor.
    pub snapshot: StateSnapshot<S>,
}

/// What one pull returned.
#[derive(Debug)]
#[non_exhaustive]
pub enum Pulled<D, S> {
    /// Matching events.
    Events(Batch<D>),
    /// Nothing matched before the deadline.
    Heartbeat(Heartbeat),
    /// Delivery was lost; the loss is already durable.
    Missed(Recovery<S>),
    /// The subscription was unsubscribed by identity.
    Closed(Closed),
}

/// Pull handle of one independent subscription; dropping it closes only this one.
#[derive(Debug)]
pub struct Feed<D, P> {
    pub(super) shared: Arc<Shared<D, P>>,
    pub(super) id: SubscriptionId,
}

impl<D, P> Feed<D, P> {
    /// Identity of this subscription.
    pub const fn id(&self) -> &SubscriptionId {
        &self.id
    }
}

impl<D, P> Drop for Feed<D, P> {
    fn drop(&mut self) {
        if let Ok(mut table) = self.shared.table.lock() {
            table.orphan(&self.id);
        }
    }
}

impl<D, P> Feed<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned + From<LossRecord>,
    P: SnapshotProducer,
{
    /// Waits for matching events and returns a heartbeat when idle until `deadline` or the
    /// configured heartbeat interval, whichever comes first.
    ///
    /// # Errors
    ///
    /// Returns `coordinator_stopped` after shutdown, or a typed persistence or
    /// snapshot failure while recovering from loss.
    pub fn next(&self, deadline: Instant) -> Result<Pulled<D, P::Sections>, SubscriptionError> {
        let deadline = Instant::now()
            .checked_add(self.shared.limits.heartbeat)
            .map_or(deadline, |beat| beat.min(deadline));
        loop {
            let mut table = self.shared.lock()?;
            if self.shared.stopped.load(Ordering::SeqCst) {
                return Err(SubscriptionError::Stopped);
            }
            let position = match table.take(&self.id)? {
                Step::Idle(position) => position,
                Step::Events(batch) => {
                    drop(table);
                    return Ok(Pulled::Events(batch));
                }
                Step::Closed(closed) => {
                    drop(table);
                    return Ok(Pulled::Closed(closed));
                }
                Step::Lost => {
                    drop(table);
                    self.shared.persist_pending()?;
                    if let Some(recovery) = self.recover()? {
                        return Ok(Pulled::Missed(recovery));
                    }
                    continue;
                }
            };
            if Instant::now() >= deadline {
                drop(table);
                return Ok(Pulled::Heartbeat(self.heartbeat(position)));
            }
            self.wait(table, deadline)?;
        }
    }

    /// Releases the table until a fan-out, close or stop wakes this consumer, or `deadline`.
    fn wait(
        &self,
        table: MutexGuard<'_, Table<D>>,
        deadline: Instant,
    ) -> Result<(), SubscriptionError> {
        let timeout = deadline.saturating_duration_since(Instant::now());
        let (table, _) = self
            .shared
            .ready
            .wait_timeout(table, timeout)
            .map_err(|_| SubscriptionError::Stopped)?;
        drop(table);
        Ok(())
    }

    /// Commits a forced-close marker for undelivered events, then closes this subscription.
    ///
    /// # Errors
    ///
    /// Returns the typed persistence failure; the marker stays pending and is retried.
    pub fn close_forced(self) -> Result<Option<LossRecord>, SubscriptionError> {
        let owed = {
            let mut table = self.shared.lock()?;
            let owed = table.find(&self.id).ok().and_then(|entry| entry.owed());
            table.orphan(&self.id);
            owed
        };
        self.shared.persist_pending()?;
        Ok(owed)
    }

    fn recover(&self) -> Result<Option<Recovery<P::Sections>>, SubscriptionError> {
        let mut table = self.shared.lock()?;
        if !table.awaiting_restart(&self.id)? {
            return Ok(None);
        }
        let snapshot = self.shared.capture()?;
        let restarted = table.restart(&self.id, snapshot.cursor.seq.get())?;
        drop(table);
        Ok(restarted.map(|lost| Recovery {
            loss: lost.record(&self.id),
            snapshot,
        }))
    }

    fn heartbeat(&self, position: Closed) -> Heartbeat {
        let identity = self.shared.identity;
        Heartbeat {
            subscription_id: self.id.clone(),
            seq: position.seq,
            scan_seq: position.scan_seq,
            machine_id: identity.machine_id(),
            runtime_id: identity.runtime_id(),
            boot_id: identity.server_boot_id(),
        }
    }
}
