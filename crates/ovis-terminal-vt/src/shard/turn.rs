//! The shard: an engine, its unparsed output and one bounded turn at a time.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::budget::{ParseBudget, PendingCap};
use super::error::ShardError;
use crate::{FeedOutcome, Vt};

#[derive(Debug)]
struct Pending {
    bytes: Arc<[u8]>,
    from: usize,
}

impl Pending {
    fn rest(&self) -> &[u8] {
        self.bytes.get(self.from..).unwrap_or_default()
    }
}

/// What one parse turn did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Turn {
    /// Bytes fed to the engine in this turn; never more than the budget.
    pub fed: usize,
    /// Queries and OSC records completed in this turn, in stream order.
    pub outcome: FeedOutcome,
    /// Bytes still waiting after this turn.
    pub remaining: usize,
    /// Wall time the engine spent on this turn.
    pub elapsed: Duration,
}

/// One terminal's engine fed in budgeted turns from a bounded queue of shared chunks.
#[derive(Debug)]
pub struct Shard {
    vt: Vt,
    budget: ParseBudget,
    cap: PendingCap,
    pending: VecDeque<Pending>,
    pending_bytes: usize,
}

impl Shard {
    /// A shard over `vt` with `budget` per turn and at most `cap` bytes waiting.
    #[must_use]
    pub const fn new(vt: Vt, budget: ParseBudget, cap: PendingCap) -> Self {
        Self {
            vt,
            budget,
            cap,
            pending: VecDeque::new(),
            pending_bytes: 0,
        }
    }

    /// Queues a chunk by reference; the bytes are never copied.
    ///
    /// # Errors
    ///
    /// Returns [`ShardError::Full`] when the chunk would pass the pending cap; nothing
    /// is queued then.
    pub fn push(&mut self, bytes: Arc<[u8]>) -> Result<(), ShardError> {
        let offered = bytes.len();
        let room = self.cap.bytes().saturating_sub(self.pending_bytes);
        if offered > room {
            return Err(ShardError::Full {
                offered,
                pending: self.pending_bytes,
                cap: self.cap.bytes(),
            });
        }
        if offered > 0 {
            self.pending_bytes = self.pending_bytes.saturating_add(offered);
            self.pending.push_back(Pending { bytes, from: 0 });
        }
        Ok(())
    }

    /// Feeds at most one budget of waiting bytes, in order.
    pub fn turn(&mut self) -> Turn {
        let started = Instant::now();
        let mut left = self.budget.bytes();
        let mut fed = 0_usize;
        let mut outcome = FeedOutcome::default();
        while left > 0 {
            let Some(front) = self.pending.front_mut() else {
                break;
            };
            let take = front.rest().len().min(left);
            let slice = front.rest().get(..take).unwrap_or_default();
            absorb(&mut outcome, self.vt.feed(slice));
            front.from = front.from.saturating_add(take);
            if front.rest().is_empty() {
                self.pending.pop_front();
            }
            left = left.saturating_sub(take);
            fed = fed.saturating_add(take);
        }
        self.pending_bytes = self.pending_bytes.saturating_sub(fed);
        Turn {
            fed,
            outcome,
            remaining: self.pending_bytes,
            elapsed: started.elapsed(),
        }
    }

    /// Bytes waiting for a turn.
    #[must_use]
    pub const fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// The engine, for reads.
    #[must_use]
    pub const fn vt(&self) -> &Vt {
        &self.vt
    }

    /// The engine, for resizes; waiting bytes are parsed after the change.
    pub const fn vt_mut(&mut self) -> &mut Vt {
        &mut self.vt
    }
}

fn absorb(into: &mut FeedOutcome, from: FeedOutcome) {
    into.queries.extend(from.queries);
    into.osc.extend(from.osc);
    into.suppressed_queries = into
        .suppressed_queries
        .saturating_add(from.suppressed_queries);
}
