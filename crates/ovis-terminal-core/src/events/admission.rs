//! FIFO admission to the pinned-snapshot budget (plan todo 79).
//!
//! Every pinned snapshot holds one of the coordinator's slots. Inspection waits for a
//! slot in arrival order instead of failing at the budget, and a fail-fast pin never
//! takes a slot while anyone is waiting, so a released slot always reaches the oldest
//! waiter. Waiting blocks on a condition variable; nothing polls.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use ovis_terminal_protocol::schema::scalar::QueuePosition;

use super::EventError;

/// Identity of one waiting request, stable while it moves up the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TicketId(u64);

#[derive(Debug, Default)]
struct Queue {
    held: usize,
    next: u64,
    waiting: VecDeque<TicketId>,
    withdrawn: Vec<TicketId>,
}

/// The coordinator's pinned-snapshot slots and the FIFO of requests waiting for one.
#[derive(Debug)]
pub struct SnapshotQueue {
    slots: usize,
    queue: Mutex<Queue>,
    turn: Condvar,
}

impl SnapshotQueue {
    pub(super) fn new(slots: usize) -> Arc<Self> {
        Arc::new(Self {
            slots,
            queue: Mutex::new(Queue::default()),
            turn: Condvar::new(),
        })
    }

    // The queue holds only counters and identities, each updated by one statement, so a
    // panic elsewhere cannot leave it inconsistent; recovering keeps slots releasable.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Joins the back of the queue; the returned ticket waits with [`QueueTicket::wait`].
    pub fn enqueue(self: &Arc<Self>) -> QueueTicket {
        let mut queue = self.lock();
        let id = TicketId(queue.next);
        queue.next = queue.next.wrapping_add(1);
        queue.waiting.push_back(id);
        drop(queue);
        QueueTicket {
            id,
            queue: Arc::clone(self),
        }
    }

    /// The ticket's one-based place in line, or `None` once granted, withdrawn or dropped.
    pub fn position(&self, ticket: TicketId) -> Option<QueuePosition> {
        let index = self
            .lock()
            .waiting
            .iter()
            .position(|waiting| *waiting == ticket)?;
        let place = u64::try_from(index).ok()?.checked_add(1)?;
        Some(QueuePosition::new(place))
    }

    /// Removes a waiting ticket; its waiter wakes with `unavailable`.
    pub fn withdraw(&self, ticket: TicketId) {
        let mut queue = self.lock();
        let before = queue.waiting.len();
        queue.waiting.retain(|waiting| *waiting != ticket);
        if queue.waiting.len() != before {
            queue.withdrawn.push(ticket);
        }
        drop(queue);
        self.turn.notify_all();
    }

    /// Takes a free slot only when no one is waiting for it.
    pub(super) fn try_acquire(self: &Arc<Self>) -> Result<SnapshotSlot, EventError> {
        let mut queue = self.lock();
        if !queue.waiting.is_empty() || queue.held >= self.slots {
            return Err(EventError::LimitExceeded);
        }
        queue.held = queue.held.saturating_add(1);
        drop(queue);
        Ok(SnapshotSlot {
            queue: Arc::clone(self),
        })
    }
}

/// A request's place in the queue; dropping it before the grant leaves the queue.
#[derive(Debug)]
#[must_use = "a ticket must wait for its slot or be dropped to leave the queue"]
pub struct QueueTicket {
    id: TicketId,
    queue: Arc<SnapshotQueue>,
}

impl QueueTicket {
    /// The identity to inspect the ticket's position by.
    pub const fn id(&self) -> TicketId {
        self.id
    }

    /// Blocks until this ticket is first in line and a slot is free, then takes it.
    ///
    /// Call from a blocking worker.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` when [`SnapshotQueue::withdraw`] removed the ticket.
    pub fn wait(self) -> Result<SnapshotSlot, EventError> {
        let shared = &self.queue;
        let mut queue = shared.lock();
        loop {
            if let Some(at) = queue.withdrawn.iter().position(|id| *id == self.id) {
                queue.withdrawn.swap_remove(at);
                return Err(EventError::AdmissionWithdrawn);
            }
            if queue.waiting.front() == Some(&self.id) && queue.held < shared.slots {
                queue.waiting.pop_front();
                queue.held = queue.held.saturating_add(1);
                drop(queue);
                // The next waiter may also fit when several slots were released at once.
                shared.turn.notify_all();
                return Ok(SnapshotSlot {
                    queue: Arc::clone(shared),
                });
            }
            queue = shared
                .turn
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for QueueTicket {
    fn drop(&mut self) {
        let mut queue = self.queue.lock();
        let before = queue.waiting.len();
        queue.waiting.retain(|waiting| *waiting != self.id);
        queue.withdrawn.retain(|withdrawn| *withdrawn != self.id);
        let left = queue.waiting.len() != before;
        drop(queue);
        if left {
            self.queue.turn.notify_all();
        }
    }
}

/// One held pinned-snapshot slot; dropping it hands the slot to the oldest waiter.
#[derive(Debug)]
pub struct SnapshotSlot {
    queue: Arc<SnapshotQueue>,
}

impl Drop for SnapshotSlot {
    fn drop(&mut self) {
        let mut queue = self.queue.lock();
        queue.held = queue.held.saturating_sub(1);
        drop(queue);
        self.queue.turn.notify_all();
    }
}
