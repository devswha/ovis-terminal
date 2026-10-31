//! The bounded lane for terminal-query replies; they never become user submissions.

use std::collections::VecDeque;

use super::{QueueLimits, Sequencer, SequencerError};

/// Reply bytes and entries that may wait for the holder.
const REPLY_LIMITS: QueueLimits = QueueLimits::new(64 << 10, 64);

/// Replies to the program's terminal queries, sent ahead of user entries in the core.
#[derive(Debug, Default)]
pub(super) struct Replies {
    queue: VecDeque<Vec<u8>>,
    bytes: usize,
    sent: Option<u64>,
}

impl Replies {
    fn push(&mut self, reply: Vec<u8>) -> Result<(), SequencerError> {
        let fits = self
            .bytes
            .checked_add(reply.len())
            .is_some_and(|total| total <= REPLY_LIMITS.bytes)
            && self.queue.len() < REPLY_LIMITS.entries;
        if !fits {
            return Err(SequencerError::Overflow {
                queued_bytes: self.bytes,
                queued_entries: self.queue.len(),
                requested: reply.len(),
                limits: REPLY_LIMITS,
            });
        }
        self.bytes = self.bytes.saturating_add(reply.len());
        self.queue.push_back(reply);
        Ok(())
    }

    /// The next reply when none is with the holder.
    pub(super) fn take(&mut self) -> Option<Vec<u8>> {
        if self.sent.is_some() {
            return None;
        }
        let reply = self.queue.pop_front()?;
        self.bytes = self.bytes.saturating_sub(reply.len());
        Some(reply)
    }

    pub(super) const fn sent(&mut self, write: u64) {
        self.sent = Some(write);
    }

    /// Whether `write` was the reply with the holder.
    pub(super) fn acknowledge(&mut self, write: u64) -> bool {
        self.sent.take_if(|sent| *sent == write).is_some()
    }

    pub(super) fn clear(&mut self) {
        self.queue.clear();
        self.bytes = 0;
        self.sent = None;
    }
}

impl Sequencer {
    /// Queues the runtime's reply to a terminal query on its own bounded lane.
    ///
    /// # Errors
    ///
    /// The readiness kinds when the stream is not live and `input_overflow` when the lane
    /// is full.
    pub fn admit_reply(&mut self, reply: Vec<u8>) -> Result<(), SequencerError> {
        self.live()?;
        self.replies.push(reply)
    }
}
