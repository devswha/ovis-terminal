//! The caller's side of a terminal actor: reservations, admissions, controls and output.

use std::sync::Arc;

use ovis_terminal_vt::{Geometry, OutputOffset};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::budget::{Budget, Reservation};
use super::error::SchedulerError;
use super::event::{OutputEvent, TerminalEvent};
use super::history::{HistoryError, HistoryRead};
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
    ReadHistory {
        offset: OutputOffset,
        max: usize,
        reply: oneshot::Sender<Result<HistoryReply, SchedulerError>>,
    },
    Stop,
}

pub(super) type HistoryReply = oneshot::Receiver<Result<HistoryRead, HistoryError>>;

/// A control request with the session control slot it holds until served.
#[derive(Debug)]
pub(super) struct Queued {
    pub control: Control,
    pub slot: Option<Reservation>,
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
    pub(super) controls: mpsc::Sender<Queued>,
    pub(super) admits: mpsc::Sender<Admit>,
    pub(super) budgets: [Arc<Budget>; 2],
    pub(super) control_slots: Arc<Budget>,
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

    /// Reads up to `max` bytes of disk history from `offset`, on the history thread.
    ///
    /// # Errors
    ///
    /// As [`TerminalHandle::cancel`], plus `invalid_params` when the terminal keeps no
    /// history or the history's own refusal (`history_gap` below the floor).
    pub async fn read_history(
        &self,
        offset: OutputOffset,
        max: usize,
    ) -> Result<HistoryRead, SchedulerError> {
        let (reply, queued) = oneshot::channel();
        self.control(Control::ReadHistory { offset, max, reply })?;
        let read = queued.await.map_err(|_| SchedulerError::ActorStopped)??;
        Ok(read.await.map_err(|_| SchedulerError::WriterStopped)??)
    }

    /// Asks the actor to stop; join its task for the report.
    ///
    /// Stop holds no session control slot, so it is accepted even when the session's
    /// control budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns `backpressure` when this terminal's control queue is full, `unavailable`
    /// when the actor stopped.
    pub fn stop(&self) -> Result<(), SchedulerError> {
        let queued = Queued {
            control: Control::Stop,
            slot: None,
        };
        self.controls
            .try_send(queued)
            .map_err(refused("control", self.controls.max_capacity()))
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

    /// Queues `control` holding one session control slot.
    fn control(&self, control: Control) -> Result<(), SchedulerError> {
        let slot = Some(self.control_slots.reserve(1)?);
        self.controls
            .try_send(Queued { control, slot })
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
