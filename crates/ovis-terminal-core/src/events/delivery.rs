//! Independent bounded delivery installed at the snapshot barrier.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::time::Duration;

use ovis_terminal_protocol::schema::ids::SubscriptionId;
pub use ovis_terminal_protocol::schema::methods::events::MissedReason;

use super::{Event, EventError, Filter, SnapshotLease, StateSnapshot};

/// Atomic snapshot or replay startup plus an independently owned delivery channel.
#[derive(Debug)]
#[non_exhaustive]
pub struct Subscription<D, S> {
    /// Stable identity of this installation.
    pub id: SubscriptionId,
    /// Present on initial startup or when the cursor could not be resumed.
    pub snapshot: Option<StateSnapshot<S>>,
    /// Typed reason a fresh snapshot replaced the requested replay.
    pub missed: Option<MissedReason>,
    /// Events committed after the returned snapshot or retained cursor.
    pub delivery: Delivery<D>,
}

/// Atomic startup with a pinned page lease instead of an inline snapshot.
#[derive(Debug)]
#[non_exhaustive]
pub struct PagedSubscription<D> {
    /// Independent subscription identity.
    pub id: SubscriptionId,
    /// Snapshot lease, present on fresh or missed startup.
    pub snapshot: Option<SnapshotLease>,
    /// Reason replay was unavailable.
    pub missed: Option<MissedReason>,
    /// Delivery begins strictly after the pinned revision.
    pub delivery: Delivery<D>,
}

#[derive(Debug)]
pub(super) struct Charge {
    pub bytes: usize,
    pub budget: Arc<AtomicUsize>,
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub(super) struct Queued<D> {
    pub event: Event<D>,
    pub _charge: Charge,
}

/// Blocking receiver intended for a dedicated delivery worker, not a Tokio executor.
#[derive(Debug)]
pub struct Delivery<D> {
    pub(super) receiver: Receiver<Queued<D>>,
    pub(super) lost: Arc<AtomicBool>,
}

impl<D> Delivery<D> {
    /// Receives one committed event with a bounded wait.
    ///
    /// # Errors
    ///
    /// Returns `missed` after overflow, or a typed receive timeout/disconnect.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Event<D>, EventError> {
        self.check()?;
        let queued = self
            .receiver
            .recv_timeout(timeout)
            .map_err(EventError::Receive)?;
        self.check()?;
        Ok(queued.event)
    }

    /// Receives only an event that is already published.
    ///
    /// # Errors
    ///
    /// Returns `missed` after overflow or a typed empty/disconnected receive failure.
    pub fn try_recv(&self) -> Result<Event<D>, EventError> {
        self.check()?;
        let queued = self.receiver.try_recv().map_err(EventError::TryReceive)?;
        self.check()?;
        Ok(queued.event)
    }

    fn check(&self) -> Result<(), EventError> {
        if self.lost.load(Ordering::SeqCst) {
            Err(EventError::Missed)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub(super) struct Sink<D> {
    pub sender: SyncSender<Queued<D>>,
    pub lost: std::sync::Weak<AtomicBool>,
    pub filter: Filter,
}

impl<D: Clone> Sink<D> {
    pub(super) fn publish(&self, event: &Event<D>, charge: Charge) {
        if let Some(lost) = self.lost.upgrade() {
            match self.sender.try_send(Queued {
                event: event.clone(),
                _charge: charge,
            }) {
                Err(TrySendError::Full(_)) => lost.store(true, Ordering::SeqCst),
                Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }
}
