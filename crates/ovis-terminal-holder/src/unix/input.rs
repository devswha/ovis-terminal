//! Per-terminal bounded input queue with write-boundary fencing.
//!
//! Input never shares a queue with resize, signal or cancel: those act on the
//! PTY or process directly, so a program that stops reading stdin blocks only
//! this queue's writer.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, PoisonError};

use crate::protocol::WriteGrant;

/// One admitted input request awaiting its PTY write.
#[derive(Debug)]
pub(super) struct QueuedInput {
    pub(super) sequence: u64,
    pub(super) grant: WriteGrant,
    pub(super) bytes: Vec<u8>,
}

/// Why an input request was not queued; a full queue hands the entry back.
#[derive(Debug)]
pub(super) enum PushRejected {
    Full { queued: usize, input: QueuedInput },
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

#[derive(Debug)]
pub(super) struct InputQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
    limit: usize,
}

impl InputQueue {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            ready: Condvar::new(),
            limit,
        }
    }

    /// Queues `input` within the byte bound.
    ///
    /// A grant with a newer fence epoch first revokes every unsent entry admitted
    /// under an older fence; bytes already written are not retracted. A grant
    /// older than the current fence is revoked on arrival and never queued.
    pub(super) fn push(&self, input: QueuedInput) -> Admission {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut fenced = Vec::new();
        if !state.closed && input.grant.fence_epoch < state.fence_epoch {
            fenced.push(input);
            return Admission {
                fenced,
                outcome: Ok(()),
            };
        }
        if input.grant.fence_epoch > state.fence_epoch {
            state.fence_epoch = input.grant.fence_epoch;
            let fence = state.fence_epoch;
            let (revoked, kept): (VecDeque<_>, VecDeque<_>) = state
                .entries
                .drain(..)
                .partition(|entry| entry.grant.fence_epoch < fence);
            state.entries = kept;
            fenced.extend(revoked);
            state.queued_bytes = state.entries.iter().map(|entry| entry.bytes.len()).sum();
        }
        let outcome = if state.closed {
            Err(PushRejected::Closed)
        } else {
            match state.queued_bytes.checked_add(input.bytes.len()) {
                Some(total) if total <= self.limit => {
                    state.queued_bytes = total;
                    state.entries.push_back(input);
                    self.ready.notify_one();
                    Ok(())
                }
                Some(_) | None => Err(PushRejected::Full {
                    queued: state.queued_bytes,
                    input,
                }),
            }
        };
        Admission { fenced, outcome }
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{InputQueue, PushRejected, QueuedInput};
    use crate::protocol::{ProcessIncarnation, WriteGrant};

    const LIMIT: usize = 128;

    fn offered(sequence: u64, (length, fence_epoch): (usize, u64)) -> QueuedInput {
        let process_incarnation = ProcessIncarnation::new(1);
        let grant = WriteGrant {
            grant_epoch: 1,
            fence_epoch,
            process_incarnation,
        };
        QueuedInput {
            sequence,
            grant,
            bytes: vec![0; length],
        }
    }

    proptest! {
        #[test]
        fn admission_matches_a_byte_bounded_fenced_model(
            offers in proptest::collection::vec((0_usize..64, 0_u64..4), 1..40),
        ) {
            let queue = InputQueue::new(LIMIT);
            let mut model: Vec<(u64, usize, u64)> = Vec::new();
            let mut fence = 0_u64;
            for (sequence, offer) in (1_u64..).zip(offers) {
                let admission = queue.push(offered(sequence, offer));
                let (length, epoch) = offer;
                fence = fence.max(epoch);
                let (kept, revoked): (Vec<_>, Vec<_>) =
                    model.into_iter().partition(|entry| entry.2 >= fence);
                model = kept;
                let mut revoked_sequences: Vec<u64> = revoked.iter().map(|entry| entry.0).collect();
                let stale = epoch < fence;
                if stale {
                    revoked_sequences.push(sequence);
                }
                let fenced: Vec<u64> = admission.fenced.iter().map(|entry| entry.sequence).collect();
                prop_assert_eq!(fenced, revoked_sequences);
                let queued: usize = model.iter().map(|entry| entry.1).sum();
                let fits = queued.saturating_add(length) <= LIMIT;
                match admission.outcome {
                    Ok(()) if stale => {}
                    Ok(()) => {
                        prop_assert!(fits);
                        model.push((sequence, length, epoch));
                    }
                    Err(PushRejected::Full { queued: reported, input }) => {
                        prop_assert!(!fits);
                        prop_assert_eq!((reported, input.sequence), (queued, sequence));
                    }
                    Err(PushRejected::Closed) => prop_assert!(false, "queue is open"),
                }
            }
        }
    }
}
