//! The ordered, bounded message stream from a holder to its adopting core.
//!
//! One FIFO keeps `Spawned`, `Output`, `InputWritten` and `Exit` in the order the
//! holder produced them, so output through a final offset always precedes its
//! exit. Holder threads never block on a slow core: a link whose backlog passes
//! its bound is closed, and the core recovers by re-adopting and reading tails.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::protocol::Message;

/// Why a core link stopped delivering messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LinkClosed {
    /// A newer adoption took ownership.
    Replaced,
    /// The core side released the link.
    Released,
    /// Undelivered messages exceeded the link bound.
    Overflow,
    /// The holder shut down.
    HolderStopped,
}

/// A message could not be received from a core link.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RecvError {
    /// No message arrived before the deadline.
    #[error("no holder message arrived before the deadline")]
    Timeout,
    /// The link is closed.
    #[error("the core link is closed ({reason:?})")]
    Closed {
        /// Reason the link closed.
        reason: LinkClosed,
    },
}

impl RecvError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Timeout => "recv_timeout",
            Self::Closed { .. } => "link_closed",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Timeout => "Wait again, or inspect the terminal record.",
            Self::Closed { .. } => "Re-adopt the holder; retained records and tails replay.",
        }
    }
}

#[derive(Debug, Default)]
struct OutboxState {
    queue: VecDeque<(Message, usize)>,
    cost: usize,
    closed: Option<LinkClosed>,
}

impl OutboxState {
    fn enqueue(&mut self, message: Message, limit: usize) -> Result<(), LinkClosed> {
        let cost = message_cost(&message);
        match (self.closed, self.cost.checked_add(cost)) {
            (Some(reason), _) => Err(reason),
            (None, Some(total)) if total <= limit => {
                self.cost = total;
                self.queue.push_back((message, cost));
                Ok(())
            }
            (None, Some(_) | None) => {
                self.close(LinkClosed::Overflow);
                Err(LinkClosed::Overflow)
            }
        }
    }

    fn close(&mut self, reason: LinkClosed) {
        if self.closed.is_none() {
            self.closed = Some(reason);
            self.queue.clear();
            self.cost = 0;
        }
    }
}

#[derive(Debug)]
pub(super) struct Outbox {
    state: Mutex<OutboxState>,
    ready: Condvar,
    limit: usize,
}

impl Outbox {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(OutboxState::default()),
            ready: Condvar::new(),
            limit,
        }
    }

    /// Appends a message; a full or closed link refuses it.
    pub(super) fn push(&self, message: Message) -> Result<(), LinkClosed> {
        let outcome = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .enqueue(message, self.limit);
        self.ready.notify_all();
        outcome
    }

    /// Closes the link; the first reason sticks and undelivered messages are dropped.
    pub(super) fn close(&self, reason: LinkClosed) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close(reason);
        self.ready.notify_all();
    }

    fn recv_until(&self, deadline: Option<Instant>) -> Result<Message, RecvError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(reason) = state.closed {
                return Err(RecvError::Closed { reason });
            }
            if let Some((message, cost)) = state.queue.pop_front() {
                state.cost = state.cost.saturating_sub(cost);
                return Ok(message);
            }
            state = match deadline {
                None => self
                    .ready
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(RecvError::Timeout);
                    }
                    self.ready
                        .wait_timeout(state, remaining)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
    }
}

const MESSAGE_OVERHEAD: usize = 256;

const fn message_cost(message: &Message) -> usize {
    let payload = match message {
        Message::Output(output) => output.bytes.len(),
        Message::Tail(tail) => tail.bytes.len(),
        Message::Input(input) => input.bytes.len(),
        Message::Welcome(welcome) => welcome.terminals.len().saturating_mul(MESSAGE_OVERHEAD),
        Message::Hello(_)
        | Message::Spawn(_)
        | Message::Spawned(_)
        | Message::SpawnFailed(_)
        | Message::InputWritten(_)
        | Message::Resize(_)
        | Message::Signal(_)
        | Message::Exit(_)
        | Message::TailRequest(_)
        | Message::Forget(_)
        | Message::Drain(_)
        | Message::Goodbye(_) => 0,
    };
    payload.saturating_add(MESSAGE_OVERHEAD)
}

/// The receiving end of a holder's message stream to one adopting core.
#[derive(Clone, Debug)]
pub struct CoreLink {
    outbox: Arc<Outbox>,
}

impl CoreLink {
    pub(super) const fn new(outbox: Arc<Outbox>) -> Self {
        Self { outbox }
    }

    pub(super) const fn outbox(&self) -> &Arc<Outbox> {
        &self.outbox
    }

    /// Blocks until the next holder message.
    ///
    /// # Errors
    ///
    /// Returns [`RecvError::Closed`] once the link is closed.
    pub fn recv(&self) -> Result<Message, RecvError> {
        self.outbox.recv_until(None)
    }

    /// Waits at most `timeout` for the next holder message.
    ///
    /// # Errors
    ///
    /// Returns [`RecvError::Timeout`] when nothing arrives in time and
    /// [`RecvError::Closed`] once the link is closed.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Message, RecvError> {
        // A deadline past the clock's range is no deadline at all.
        self.outbox.recv_until(Instant::now().checked_add(timeout))
    }

    /// Releases the link; the holder keeps every terminal and its records.
    pub fn release(&self) {
        self.outbox.close(LinkClosed::Released);
    }
}
