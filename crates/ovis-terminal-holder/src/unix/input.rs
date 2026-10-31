//! Per-terminal bounded input queue with write-boundary fencing.
//!
//! Input never shares a queue with resize, signal or cancel: those act on the
//! PTY or process directly, so a program that stops reading stdin blocks only
//! this queue's writer. Both the queued bytes and the queued entries are bounded,
//! so empty requests cannot grow the queue either.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, PoisonError};

use super::config::QueueLimits;
use crate::protocol::WriteGrant;

/// One admitted input request awaiting its PTY write.
#[derive(Debug)]
pub(super) struct QueuedInput {
    pub(super) sequence: u64,
    pub(super) grant: WriteGrant,
    pub(super) bytes: Vec<u8>,
}

/// What is waiting for the PTY at one instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct QueueUsage {
    pub(super) bytes: usize,
    pub(super) entries: usize,
}

/// Why an input request was not queued; a full queue hands the entry back.
#[derive(Debug)]
pub(super) enum PushRejected {
    Full {
        queued: QueueUsage,
        input: QueuedInput,
    },
    Closed,
}

/// Result of offering input: entries revoked by a newer fence, plus the outcome.
#[derive(Debug)]
pub(super) struct Admission {
    pub(super) fenced: Vec<QueuedInput>,
    pub(super) outcome: Result<(), PushRejected>,
}

/// An entry handed to the writer with the fence epoch current at that moment.
#[derive(Debug)]
pub(super) struct Dequeued {
    pub(super) input: QueuedInput,
    pub(super) fence_epoch: u64,
}

#[derive(Debug, Default)]
struct QueueState {
    entries: VecDeque<QueuedInput>,
    queued_bytes: usize,
    fence_epoch: u64,
    closed: bool,
}

impl QueueState {
    fn usage(&self) -> QueueUsage {
        QueueUsage {
            bytes: self.queued_bytes,
            entries: self.entries.len(),
        }
    }

    fn admit(&mut self, input: QueuedInput, limits: QueueLimits) -> Admission {
        let mut fenced = Vec::new();
        if !self.closed && input.grant.fence_epoch < self.fence_epoch {
            fenced.push(input);
            return Admission {
                fenced,
                outcome: Ok(()),
            };
        }
        if input.grant.fence_epoch > self.fence_epoch {
            self.fence_epoch = input.grant.fence_epoch;
            let fence = self.fence_epoch;
            let (revoked, kept): (VecDeque<_>, VecDeque<_>) = self
                .entries
                .drain(..)
                .partition(|entry| entry.grant.fence_epoch < fence);
            self.entries = kept;
            fenced.extend(revoked);
            self.queued_bytes = self.entries.iter().map(|entry| entry.bytes.len()).sum();
        }
        let usage = self.usage();
        let bytes = usage
            .bytes
            .checked_add(input.bytes.len())
            .filter(|total| *total <= limits.bytes);
        let outcome = match (self.closed, bytes) {
            (true, _) => Err(PushRejected::Closed),
            (false, Some(total)) if usage.entries < limits.entries => {
                self.queued_bytes = total;
                self.entries.push_back(input);
                Ok(())
            }
            (false, _) => Err(PushRejected::Full {
                queued: usage,
                input,
            }),
        };
        Admission { fenced, outcome }
    }
}

#[derive(Debug)]
pub(super) struct InputQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
    limits: QueueLimits,
}

impl InputQueue {
    pub(super) fn new(limits: QueueLimits) -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            ready: Condvar::new(),
            limits,
        }
    }

    pub(super) const fn limits(&self) -> QueueLimits {
        self.limits
    }

    /// Queues `input` within the byte and entry bounds.
    ///
    /// A grant with a newer fence epoch first revokes every unsent entry admitted
    /// under an older fence; bytes already written are not retracted. A grant
    /// older than the current fence is revoked on arrival and never queued.
    pub(super) fn push(&self, input: QueuedInput) -> Admission {
        let admission = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(input, self.limits);
        self.ready.notify_one();
        admission
    }

    /// Blocks until an entry is available; `None` once closed and empty.
    pub(super) fn pop(&self) -> Option<Dequeued> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(input) = state.entries.pop_front() {
                state.queued_bytes = state.queued_bytes.saturating_sub(input.bytes.len());
                let fence_epoch = state.fence_epoch;
                return Some(Dequeued { input, fence_epoch });
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Fence epoch at this instant, rechecked immediately before a write.
    pub(super) fn fence_epoch(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fence_epoch
    }

    /// Reports whether admission stopped because the process exited.
    pub(super) fn is_closed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed
    }

    /// Stops admission; queued entries are still handed to the writer, which
    /// acknowledges them unwritten.
    pub(super) fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed = true;
        self.ready.notify_all();
    }
}
