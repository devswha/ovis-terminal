//! Per-terminal input bounded in bytes and entries, rejected before anything is queued.
//!
//! Resize and kill never enter this queue, so a program that stops reading stdin
//! blocks only the writer of its own input.

use std::collections::VecDeque;

use thiserror::Error;

use super::InputSequence;

/// Input that may wait for the pseudoconsole; both bounds apply to every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputLimits {
    bytes: usize,
    entries: usize,
}

impl InputLimits {
    /// 16 MiB in at most 4096 requests, the Unix holder's bound.
    pub const DEFAULT: Self = Self::new(16 << 20, 4096);

    /// Explicit bounds.
    #[must_use]
    pub const fn new(bytes: usize, entries: usize) -> Self {
        Self { bytes, entries }
    }
}

impl Default for InputLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What waits for the pseudoconsole at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueUsage {
    /// Queued bytes.
    pub bytes: usize,
    /// Queued requests.
    pub entries: usize,
}

/// Why input was not queued; none of its bytes were accepted.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputRejected {
    /// The request does not fit behind what is already queued.
    #[error("{requested} bytes do not fit behind {} bytes in {} queued requests", queued.bytes, queued.entries)]
    Overflow {
        /// What was queued when the request arrived.
        queued: QueueUsage,
        /// Bytes in the rejected request.
        requested: usize,
        /// The terminal's bounds.
        limits: InputLimits,
    },
    /// The program has exited; its input queue is closed.
    #[error("the program has exited and takes no more input")]
    Closed,
    /// The sequence space of this terminal is used up.
    #[error("the input sequence cannot advance further")]
    Exhausted,
}

impl InputRejected {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Overflow { .. } => "input_overflow",
            Self::Closed => "process_exited",
            Self::Exhausted => "counter_exhausted",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Overflow { .. } => "wait for queued input to be written before sending more",
            Self::Closed => "read the exit; the program can no longer be driven",
            Self::Exhausted => "relaunch the terminal; its input sequence space is used up",
        }
    }
}

/// Input requests in arrival order, each written whole or reported unwritten.
#[derive(Debug)]
pub struct InputQueue {
    entries: VecDeque<(InputSequence, Vec<u8>)>,
    queued_bytes: usize,
    limits: InputLimits,
    last: u64,
    closed: bool,
}

impl InputQueue {
    /// An empty, open queue.
    #[must_use]
    pub const fn new(limits: InputLimits) -> Self {
        Self {
            entries: VecDeque::new(),
            queued_bytes: 0,
            limits,
            last: 0,
            closed: false,
        }
    }

    /// What is queued now.
    #[must_use]
    pub fn usage(&self) -> QueueUsage {
        QueueUsage {
            bytes: self.queued_bytes,
            entries: self.entries.len(),
        }
    }

    /// Queues `bytes` within both bounds and returns its sequence.
    ///
    /// # Errors
    ///
    /// [`InputRejected::Closed`] after [`Self::close`], [`InputRejected::Overflow`] when
    /// either bound would be exceeded, and [`InputRejected::Exhausted`] when the
    /// sequence cannot advance. A rejected request leaves the queue unchanged.
    pub fn push(&mut self, bytes: Vec<u8>) -> Result<InputSequence, InputRejected> {
        if self.closed {
            return Err(InputRejected::Closed);
        }
        let total = self
            .queued_bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= self.limits.bytes);
        let Some(total) = total.filter(|_| self.entries.len() < self.limits.entries) else {
            return Err(InputRejected::Overflow {
                queued: self.usage(),
                requested: bytes.len(),
                limits: self.limits,
            });
        };
        let next = self.last.checked_add(1).ok_or(InputRejected::Exhausted)?;
        self.last = next;
        self.queued_bytes = total;
        self.entries.push_back((InputSequence(next), bytes));
        Ok(InputSequence(next))
    }

    /// The oldest queued request, removed from the queue.
    pub fn pop(&mut self) -> Option<(InputSequence, Vec<u8>)> {
        let entry = self.entries.pop_front()?;
        self.queued_bytes = self.queued_bytes.saturating_sub(entry.1.len());
        Some(entry)
    }

    /// Whether [`Self::close`] has run.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    /// Stops admission and returns the sequences that will never be written.
    pub fn close(&mut self) -> Vec<InputSequence> {
        self.closed = true;
        self.queued_bytes = 0;
        self.entries
            .drain(..)
            .map(|(sequence, _)| sequence)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{InputLimits, InputQueue, InputRejected, InputSequence};

    proptest! {
        #[test]
        fn the_queue_never_exceeds_either_bound_and_writes_in_admission_order(
            requests in proptest::collection::vec((0_usize..40, any::<bool>()), 0..60),
            byte_limit in 0_usize..120,
            entry_limit in 0_usize..8,
        ) {
            let mut queue = InputQueue::new(InputLimits::new(byte_limit, entry_limit));
            let mut admitted = Vec::new();
            let mut written = Vec::new();
            for (length, pop) in requests {
                let before = queue.usage();
                match queue.push(vec![7; length]) {
                    Ok(sequence) => admitted.push(sequence),
                    Err(InputRejected::Overflow { queued, requested, .. }) => {
                        prop_assert_eq!(queued, before);
                        prop_assert_eq!(queue.usage(), before, "a rejection queues nothing");
                        let over = before.bytes.checked_add(requested).is_none_or(|total| total > byte_limit);
                        prop_assert!(over || before.entries >= entry_limit);
                    }
                    Err(other) => prop_assert!(false, "unexpected {other:?}"),
                }
                prop_assert!(queue.usage().bytes <= byte_limit && queue.usage().entries <= entry_limit);
                if pop && let Some((sequence, _)) = queue.pop() {
                    written.push(sequence);
                }
            }
            let unwritten = queue.close();
            written.extend(unwritten);
            prop_assert_eq!(written, admitted, "every admitted request is written or reported unwritten, in order");
            prop_assert_eq!(queue.push(Vec::new()), Err(InputRejected::Closed));
        }
    }

    #[test]
    fn an_empty_request_still_counts_against_the_entry_bound() {
        let mut queue = InputQueue::new(InputLimits::new(0, 1));
        assert_eq!(queue.push(Vec::new()).map(InputSequence::get), Ok(1));
        let error = queue.push(Vec::new()).expect_err("entry bound");
        assert_eq!(error.kind(), "input_overflow");
        assert!(!error.to_string().contains("input_overflow"));
    }
}
