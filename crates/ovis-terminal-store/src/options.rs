//! Advertised limits and commit-barrier observation for an opened store.

use std::fmt;
use std::sync::Arc;

/// A point in the writer's commit sequence where fault injection can observe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommitBarrier {
    /// Every row of the mutation is written inside the transaction; COMMIT has not run.
    Applied,
    /// COMMIT returned; the reply has not been sent.
    Committed,
}

type BarrierHook = Arc<dyn Fn(CommitBarrier) + Send + Sync>;

/// Bounds advertised to clients; eviction past them advances a visible floor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreLimits {
    /// Largest resource value, receipt result or event body, in bytes.
    pub max_value_bytes: usize,
    /// Events retained before the oldest are evicted behind the event floor.
    pub max_events: u64,
    /// Receipts retained before the oldest are evicted behind the receipt floor.
    pub max_receipts: u64,
    /// Mutations that may wait for the writer before submission is refused.
    pub writer_queue: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_value_bytes: 1 << 20,
            max_events: 10_000,
            max_receipts: 100_000,
            writer_queue: 64,
        }
    }
}

impl StoreLimits {
    /// Sets the largest stored payload in bytes.
    #[must_use]
    pub const fn with_max_value_bytes(mut self, bytes: usize) -> Self {
        self.max_value_bytes = bytes;
        self
    }

    /// Sets how many events are retained.
    #[must_use]
    pub const fn with_max_events(mut self, events: u64) -> Self {
        self.max_events = events;
        self
    }

    /// Sets how many receipts are retained.
    #[must_use]
    pub const fn with_max_receipts(mut self, receipts: u64) -> Self {
        self.max_receipts = receipts;
        self
    }

    /// Sets the writer queue capacity.
    #[must_use]
    pub const fn with_writer_queue(mut self, capacity: usize) -> Self {
        self.writer_queue = capacity;
        self
    }
}

/// How a store is opened.
#[derive(Clone, Default)]
pub struct StoreOptions {
    pub(crate) limits: StoreLimits,
    barrier: Option<BarrierHook>,
}

impl StoreOptions {
    /// Uses `limits` instead of the defaults.
    #[must_use]
    pub const fn with_limits(mut self, limits: StoreLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Calls `hook` on the writer thread at each [`CommitBarrier`].
    #[must_use]
    pub fn with_barrier(mut self, hook: impl Fn(CommitBarrier) + Send + Sync + 'static) -> Self {
        self.barrier = Some(Arc::new(hook));
        self
    }

    pub(crate) fn reach(&self, barrier: CommitBarrier) {
        if let Some(hook) = &self.barrier {
            hook(barrier);
        }
    }
}

impl fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreOptions")
            .field("limits", &self.limits)
            .field("barrier_hook", &self.barrier.is_some())
            .finish()
    }
}
