//! The subscription hub: one fan-out worker and independently owned subscriptions.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::shared::{Shared, head};
use super::table::Table;
use super::{Closed, LossRecord, QueueLimits, Selector, SubscriptionError, pump};
use crate::events::{Coordinator, EventData, SnapshotProducer};
use crate::registry::RuntimeIdentity;

type Worker = JoinHandle<Result<(), SubscriptionError>>;

/// Owns every subscription of one core and the worker that fans commits out to them.
///
/// The worker holds the only coordinator delivery and copies each committed event into
/// the matching subscriptions' own bounded queues, so a stalled, replaced or closed
/// subscription never affects another one.
#[derive(Debug)]
pub struct Subscriptions<D, P> {
    pub(super) shared: Arc<Shared<D, P>>,
    worker: Option<Worker>,
}

impl<D, P> Subscriptions<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned + From<LossRecord> + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    /// Installs the fan-out delivery at the current revision and starts its worker.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` when `identity` is not the coordinator's or the bounds are
    /// invalid, `unavailable` when the worker cannot start, or a coordinator failure.
    pub fn start(
        core: Arc<Coordinator<D, P>>,
        identity: RuntimeIdentity,
        limits: QueueLimits,
    ) -> Result<Self, SubscriptionError> {
        let limits = limits.checked()?;
        let fanout = core.subscribe(None)?;
        let snapshot = fanout.snapshot.as_ref();
        if snapshot.is_none_or(|snapshot| {
            snapshot.cursor.machine_id != identity.machine_id()
                || snapshot.cursor.runtime_id != identity.runtime_id()
        }) {
            return Err(SubscriptionError::IdentityMismatch);
        }
        let table = Table::new(head(snapshot)?, limits);
        let shared = Arc::new(Shared::new(core, identity, table));
        let weak: Weak<Shared<D, P>> = Arc::downgrade(&shared);
        let delivery = fanout.delivery;
        let worker = std::thread::Builder::new()
            .name("ovis-subscriptions".into())
            .spawn(move || pump::run(&weak, delivery))
            .map_err(SubscriptionError::Spawn)?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
}

impl<D, P> Subscriptions<D, P> {
    /// The bounds every subscription of this hub runs under.
    pub fn limits(&self) -> QueueLimits {
        self.shared.limits
    }

    pub(super) fn stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::SeqCst)
    }

    /// Replaces one subscription's filter in place; delivery continues across the change.
    ///
    /// Events at or below the returned sequence minus one were judged by the previous
    /// filter and are not re-filtered; every later event is judged by the new one.
    ///
    /// # Errors
    ///
    /// Returns `not_found` for an unknown or closed subscription.
    pub fn update(
        &self,
        id: &SubscriptionId,
        selector: Selector,
    ) -> Result<EventSeq, SubscriptionError> {
        self.shared.lock()?.update(id, selector)
    }

    /// Closes one subscription; its consumer's next pull reports where it stopped.
    ///
    /// # Errors
    ///
    /// Returns `not_found` for an unknown or already closed subscription.
    pub fn unsubscribe(&self, id: &SubscriptionId) -> Result<Closed, SubscriptionError> {
        let closed = self.shared.lock()?.close(id)?;
        self.shared.ready.notify_all();
        Ok(closed)
    }

    /// Stops and joins the worker; consumers then receive `coordinator_stopped`.
    ///
    /// # Errors
    ///
    /// Returns the worker's own failure, such as a durable loss write that failed.
    pub fn shutdown(mut self) -> Result<(), SubscriptionError> {
        self.shared.stop();
        match self.worker.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(SubscriptionError::Stopped),
            None => Ok(()),
        }
    }
}
