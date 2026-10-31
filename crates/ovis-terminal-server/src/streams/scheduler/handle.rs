//! The caller's side of a terminal actor: reservations, admissions, controls and output.

use std::sync::Arc;

use ovis_terminal_vt::Geometry;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::budget::ByteBudget;
use super::error::SchedulerError;
use super::event::{OutputEvent, TerminalEvent};
use super::input::{CancelOutcome, Reserved, Ticket};

/// An admitted input entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Receipt {
    /// Admission order on the terminal.
    pub ticket: Ticket,
    /// Entries queued ahead of the writer, this one included.
    pub queued: usize,
}

#[derive(Debug)]
pub(super) enum Control {
    Cancel {
        ticket: Ticket,
        reply: oneshot::Sender<CancelOutcome>,
    },
    Resize {
        geometry: Geometry,
        reply: oneshot::Sender<()>,
    },
    Stop,
}

#[derive(Debug)]
pub(super) struct Admit {
    pub bytes: Vec<u8>,
    pub held: Reserved,
    pub reply: oneshot::Sender<Receipt>,
}

fn refused<T>(
    lane: &'static str,
    capacity: usize,
) -> impl FnOnce(TrySendError<T>) -> SchedulerError {
    move |error| match error {
        TrySendError::Full(_) => SchedulerError::QueueFull { lane, capacity },
        TrySendError::Closed(_) => SchedulerError::ActorStopped,
    }
}

/// Requests to one terminal actor; control requests have their own reserved queue.
#[derive(Debug, Clone)]
pub struct TerminalHandle {
    pub(super) controls: mpsc::Sender<Control>,
    pub(super) admits: mpsc::Sender<Admit>,
    pub(super) budgets: [Arc<ByteBudget>; 2],
    pub(super) events: broadcast::Sender<TerminalEvent>,
    pub(super) output: broadcast::Sender<OutputEvent>,
}

impl TerminalHandle {
    /// Reserves `bytes` against the terminal, then the session, input budget.
    ///
    /// Call it with the decoded length before allocating the bytes.
    ///
    /// # Errors
    ///
    /// Returns `input_overflow` naming the budget that refused; nothing is held then.
    pub fn reserve(&self, bytes: u64) -> Result<Reserved, SchedulerError> {
        let [terminal, session] = &self.budgets;
        let terminal = terminal.reserve(bytes)?;
        let session = session.reserve(bytes)?;
        Ok(Reserved {
            holds: [terminal, session],
        })
    }

    /// Queues reserved bytes for writing and returns once the actor admitted them.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` when `bytes` exceed the reservation, `backpressure` when
    /// the admission queue is full and `unavailable` when the actor stopped.
    pub async fn admit(&self, held: Reserved, bytes: Vec<u8>) -> Result<Receipt, SchedulerError> {
        let offered = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if offered > held.bytes() {
            return Err(SchedulerError::OverReservation {
                reserved: held.bytes(),
                offered,
            });
        }
        let (reply, receipt) = oneshot::channel();
        self.admits
            .try_send(Admit { bytes, held, reply })
            .map_err(refused("admission", self.admits.max_capacity()))?;
        receipt.await.map_err(|_| SchedulerError::ActorStopped)
    }

    /// Withdraws a queued entry, or reports that it is already being written.
    ///
    /// # Errors
    ///
    /// Returns `backpressure` when the control queue is full, `unavailable` when the
    /// actor stopped.
    pub async fn cancel(&self, ticket: Ticket) -> Result<CancelOutcome, SchedulerError> {
        let (reply, outcome) = oneshot::channel();
        self.control(Control::Cancel { ticket, reply })?;
        outcome.await.map_err(|_| SchedulerError::ActorStopped)
    }

    /// Resizes the terminal's screen.
    ///
    /// # Errors
    ///
    /// As [`TerminalHandle::cancel`].
    pub async fn resize(&self, geometry: Geometry) -> Result<(), SchedulerError> {
        let (reply, done) = oneshot::channel();
        self.control(Control::Resize { geometry, reply })?;
        done.await.map_err(|_| SchedulerError::ActorStopped)
    }

    /// Asks the actor to stop; join its task for the report.
    ///
    /// # Errors
    ///
    /// As [`TerminalHandle::cancel`].
    pub fn stop(&self) -> Result<(), SchedulerError> {
        self.control(Control::Stop)
    }

    /// Committed control events from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.events.subscribe()
    }

    /// Output chunks and parse progress from now on.
    #[must_use]
    pub fn subscribe_output(&self) -> broadcast::Receiver<OutputEvent> {
        self.output.subscribe()
    }

    /// Input bytes the terminal holds reserved.
    #[must_use]
    pub fn input_held(&self) -> u64 {
        let [terminal, _] = &self.budgets;
        terminal.held()
    }

    fn control(&self, control: Control) -> Result<(), SchedulerError> {
        self.controls
            .try_send(control)
            .map_err(refused("control", self.controls.max_capacity()))
    }
}

/// Where a pump hands program output to its terminal actor.
#[derive(Debug, Clone)]
pub struct OutputFeed {
    pub(super) chunks: mpsc::Sender<Arc<[u8]>>,
    pub(super) max_chunk: usize,
}

impl OutputFeed {
    /// Hands `bytes` over, waiting while the actor's output queue is full.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` when the actor stopped.
    pub async fn send(&self, bytes: &[u8]) -> Result<(), SchedulerError> {
        for piece in bytes.chunks(self.max_chunk) {
            self.chunks
                .send(Arc::from(piece))
                .await
                .map_err(|_| SchedulerError::ActorStopped)?;
        }
        Ok(())
    }

    /// Like [`OutputFeed::send`], blocking the calling thread; for pump threads only.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` when the actor stopped.
    pub fn blocking_send(&self, bytes: &[u8]) -> Result<(), SchedulerError> {
        for piece in bytes.chunks(self.max_chunk) {
            self.chunks
                .blocking_send(Arc::from(piece))
                .map_err(|_| SchedulerError::ActorStopped)?;
        }
        Ok(())
    }
}
