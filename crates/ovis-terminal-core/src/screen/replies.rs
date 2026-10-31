//! The internal holder lane for terminal-query replies: bounded, separate
//! from user input, and drained exactly once.

use std::collections::VecDeque;

use ovis_terminal_vt::{ClientFact, OutputOffset, QueryOutcome, TerminalQuery};
use serde::Serialize;

/// One query the program asked, located just past its final byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum QueryReply {
    /// Bytes the core writes to the program once, on the internal lane.
    Reply {
        /// Offset just past the query.
        offset: OutputOffset,
        /// Reply bytes.
        bytes: Vec<u8>,
    },
    /// The answer needs a fact only an attached client has.
    NeedsClientFact {
        /// Offset just past the query.
        offset: OutputOffset,
        /// The missing fact.
        fact: ClientFact,
    },
}

/// Replies taken from the lane, plus how many overflowed since the last take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ReplyBatch {
    /// Replies in stream order.
    pub replies: Vec<QueryReply>,
    /// Replies refused because the lane was full.
    pub dropped: u32,
}

#[derive(Debug)]
pub(super) struct ReplyLane {
    queue: VecDeque<QueryReply>,
    capacity: usize,
    dropped: u32,
}

impl ReplyLane {
    pub(super) const fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            capacity,
            dropped: 0,
        }
    }

    /// Queues the query's reply; returns whether it fit.
    pub(super) fn push(&mut self, query: TerminalQuery) -> bool {
        let reply = match query.outcome {
            QueryOutcome::Reply(bytes) => QueryReply::Reply {
                offset: query.offset,
                bytes,
            },
            QueryOutcome::NeedsClientFact(fact) => QueryReply::NeedsClientFact {
                offset: query.offset,
                fact,
            },
            // A query kind added to the VT engine later has no reply route yet;
            // it is counted, never silently answered.
            _ => {
                self.dropped = self.dropped.saturating_add(1);
                return false;
            }
        };
        if self.queue.len() >= self.capacity {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.queue.push_back(reply);
        true
    }

    pub(super) fn take(&mut self) -> ReplyBatch {
        ReplyBatch {
            replies: self.queue.drain(..).collect(),
            dropped: std::mem::take(&mut self.dropped),
        }
    }
}
