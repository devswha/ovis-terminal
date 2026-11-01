//! State shared by the hub, its fan-out worker and every consumer handle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::fanout::Offered;
use super::table::Table;
use super::{LossRecord, QueueLimits, SubscriptionError, loss};
use crate::events::{Coordinator, Delivery, Event, EventData, SnapshotProducer, StateSnapshot};
use crate::health::Health;
use crate::registry::RuntimeIdentity;

#[derive(Debug)]
pub(super) struct Shared<D, P> {
    pub core: Arc<Coordinator<D, P>>,
    pub identity: RuntimeIdentity,
    pub limits: QueueLimits,
    pub table: Mutex<Table<D>>,
    pub ready: Condvar,
    pub stopped: AtomicBool,
    pub health: OnceLock<Arc<Health>>,
    persisting: Mutex<()>,
}

impl<D, P> Shared<D, P> {
    pub(super) const fn new(
        core: Arc<Coordinator<D, P>>,
        identity: RuntimeIdentity,
        table: Table<D>,
    ) -> Self {
        Self {
            core,
            identity,
            limits: table.limits,
            table: Mutex::new(table),
            ready: Condvar::new(),
            stopped: AtomicBool::new(false),
            health: OnceLock::new(),
            persisting: Mutex::new(()),
        }
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Table<D>>, SubscriptionError> {
        self.table.lock().map_err(|_| SubscriptionError::Stopped)
    }

    /// Marks the fan-out worker gone and wakes every waiting consumer.
    pub(super) fn stop(&self) {
        let guard = self.table.lock();
        self.stopped.store(true, Ordering::SeqCst);
        drop(guard);
        self.ready.notify_all();
    }
}

impl<D, P> Shared<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned + From<LossRecord>,
    P: SnapshotProducer,
{
    /// Commits every loss not yet durable, then records it as persisted.
    ///
    /// Persisters are serialized so a loss is written once even when the worker and a
    /// consumer reach it together.
    pub(super) fn persist_pending(&self) -> Result<Vec<LossRecord>, SubscriptionError> {
        let _serialized = self
            .persisting
            .lock()
            .map_err(|_| SubscriptionError::Stopped)?;
        let records = self.lock()?.unpersisted();
        let Some(commit) = loss::commit(self.identity.machine_id(), records.clone()) else {
            return Ok(records);
        };
        self.core.commit(commit)?;
        self.lock()?.persisted(&records);
        Ok(records)
    }

    pub(super) fn owes(&self) -> Result<bool, SubscriptionError> {
        Ok(self.lock()?.owes())
    }

    /// Fans one committed event out, then makes any loss it caused durable.
    pub(super) fn deliver(&self, event: &Event<D>) -> Result<(), SubscriptionError> {
        let bytes = serde_json::to_vec(event)
            .map_err(crate::events::EventError::from)?
            .len();
        let lossy = {
            let mut table = self.lock()?;
            table.fan_out(Offered { event, bytes });
            table.owes()
        };
        self.ready.notify_all();
        if lossy {
            self.persist_pending()?;
        }
        Ok(())
    }

    /// Replaces an overflowed worker delivery: every active subscription loses from the
    /// last fanned-out sequence and later recovers with its own fresh snapshot.
    pub(super) fn refill(&self) -> Result<Delivery<D>, SubscriptionError> {
        let fresh = {
            let mut table = self.lock()?;
            let fresh = self.core.subscribe(None)?;
            table.lose_all(super::LossCause::FanoutOverflow);
            table.head = head(fresh.snapshot.as_ref())?;
            fresh
        };
        self.ready.notify_all();
        self.persist_pending()?;
        Ok(fresh.delivery)
    }

    /// Captures a complete snapshot under the caller's table lock, so its revision is at
    /// or after every event already fanned out.
    pub(super) fn capture(&self) -> Result<StateSnapshot<P::Sections>, SubscriptionError> {
        self.core
            .subscribe(None)?
            .snapshot
            .ok_or(SubscriptionError::Stopped)
    }
}

pub(super) fn head<S>(snapshot: Option<&StateSnapshot<S>>) -> Result<u64, SubscriptionError> {
    snapshot
        .map(|snapshot| snapshot.cursor.seq.get())
        .ok_or(SubscriptionError::Stopped)
}
