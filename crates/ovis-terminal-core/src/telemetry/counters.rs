/// One typed counter, bounded to avoid unbounded health state.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Counter {
    /// Active output streams.
    ActiveStreams,
    /// Bytes queued for consumers.
    QueueBytes,
    /// Events not yet consumed.
    EventLag,
    /// Output ranges dropped or resynchronized.
    DroppedRanges,
    /// RPC calls observed by this foundation.
    RpcCalls,
}

/// Fixed-size counters for later read-only health queries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Counters {
    active_streams: u64,
    queue_bytes: u64,
    event_lag: u64,
    dropped_ranges: u64,
    rpc_calls: u64,
}

impl Counters {
    /// Adds a delta and saturates at the published bound.
    pub fn add(&mut self, counter: Counter, amount: u64) {
        let value = match counter {
            Counter::ActiveStreams => &mut self.active_streams,
            Counter::QueueBytes => &mut self.queue_bytes,
            Counter::EventLag => &mut self.event_lag,
            Counter::DroppedRanges => &mut self.dropped_ranges,
            Counter::RpcCalls => &mut self.rpc_calls,
        };
        *value = value.saturating_add(amount).min(1_000_000);
    }

    /// Returns the current value for one counter.
    pub const fn get(&self, counter: Counter) -> u64 {
        match counter {
            Counter::ActiveStreams => self.active_streams,
            Counter::QueueBytes => self.queue_bytes,
            Counter::EventLag => self.event_lag,
            Counter::DroppedRanges => self.dropped_ranges,
            Counter::RpcCalls => self.rpc_calls,
        }
    }
}
