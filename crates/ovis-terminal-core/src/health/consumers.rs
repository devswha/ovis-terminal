use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use ovis_terminal_protocol::domain::health::{Backlog, ConsumerLoss, ConsumerRef, LossRange};
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::methods::server::ConsumerQueue;
use ovis_terminal_protocol::schema::scalar::{ByteCount, Count, DurationMs, TimestampMs};

use super::HealthError;

/// Most consumers tracked at once; a read copies at most this many rows.
pub const MAX_CONSUMERS: usize = 1024;
/// Most dropped ranges retained per consumer; older ones are only counted.
pub const MAX_LOSSES: usize = 16;
/// Most queue marks kept for lag; further writes extend the newest mark, which keeps the
/// oldest time and so can only overstate the lag.
const MAX_MARKS: usize = 64;

#[derive(Debug, Default)]
struct Flow {
    queued: u64,
    written: u64,
    /// `(cumulative end offset, commit time)` of unwritten items, oldest first.
    marks: VecDeque<(u64, TimestampMs)>,
    losses: VecDeque<ConsumerLoss>,
    omitted: u64,
}

impl Flow {
    fn backlog(&self, now: TimestampMs) -> Backlog {
        let lag = self
            .marks
            .front()
            .map_or(0, |(_, at)| now.get().saturating_sub(at.get()));
        Backlog::new(
            (
                ByteCount::new(self.queued.saturating_sub(self.written)),
                DurationMs::new(lag),
            ),
            (
                self.losses.iter().copied().collect(),
                Count::new(self.omitted),
            ),
        )
    }

    fn remember(&mut self, loss: ConsumerLoss) {
        if self.losses.len() == MAX_LOSSES {
            self.losses.pop_front();
            self.omitted = self.omitted.saturating_add(1);
        }
        self.losses.push_back(loss);
    }
}

#[derive(Debug)]
struct Line {
    client_id: ClientId,
    consumer: ConsumerRef,
    flow: Mutex<Flow>,
}

impl Line {
    fn flow(&self) -> MutexGuard<'_, Flow> {
        self.flow.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The producer's handle on one consumer's row; dropping it removes the row.
#[derive(Debug)]
pub struct ConsumerGauge(Arc<Line>);

impl ConsumerGauge {
    /// `bytes` more were queued for the consumer, the oldest committed at `committed`.
    pub fn queued(&self, bytes: u64, committed: TimestampMs) {
        let mut flow = self.0.flow();
        flow.queued = flow.queued.saturating_add(bytes);
        let end = flow.queued;
        let full = flow.marks.len() == MAX_MARKS;
        match flow.marks.back_mut() {
            Some(last) if full => last.0 = end,
            _ => flow.marks.push_back((end, committed)),
        }
    }

    /// `bytes` were written to the consumer's transport.
    pub fn written(&self, bytes: u64) {
        let mut flow = self.0.flow();
        flow.written = flow.written.saturating_add(bytes).min(flow.queued);
        let written = flow.written;
        while flow.marks.front().is_some_and(|(end, _)| *end <= written) {
            flow.marks.pop_front();
        }
        drop(flow);
    }

    /// The consumer missed `range`.
    pub fn lost(&self, range: LossRange) {
        self.0.flow().remember(ConsumerLoss::dropped(range));
    }

    /// A snapshot or resync replaced `range`; an unretained range is recorded resynced.
    pub fn resynced(&self, range: LossRange) {
        let mut flow = self.0.flow();
        match flow.losses.iter_mut().find(|loss| loss.range == range) {
            Some(loss) => loss.resynced = true,
            None => flow.remember(ConsumerLoss::replaced(range)),
        }
    }
}

/// What one read of the table saw.
#[derive(Debug)]
pub(super) struct Rows {
    pub(super) streams: u32,
    pub(super) subscriptions: u32,
    pub(super) consumers: Vec<ConsumerQueue>,
    pub(super) untracked: Count,
}

/// The bounded table of consumers producers report into.
#[derive(Debug, Default)]
pub struct Consumers {
    lines: Mutex<Vec<Weak<Line>>>,
    untracked: AtomicU64,
}

impl Consumers {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn live(&self) -> Vec<Arc<Line>> {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        lines.retain(|line| line.strong_count() > 0);
        lines.iter().filter_map(Weak::upgrade).collect()
    }

    /// Starts reporting for `consumer` of `client_id`.
    ///
    /// # Errors
    ///
    /// [`HealthError::AlreadyTracked`] while another gauge reports for `consumer`, and
    /// [`HealthError::ConsumerCapacity`] when [`MAX_CONSUMERS`] are tracked; the refused
    /// consumer is counted as untracked.
    pub fn track(
        &self,
        client_id: ClientId,
        consumer: ConsumerRef,
    ) -> Result<ConsumerGauge, HealthError> {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        lines.retain(|line| line.strong_count() > 0);
        let tracked = lines.iter().filter_map(Weak::upgrade);
        if tracked.clone().any(|line| line.consumer == consumer) {
            return Err(HealthError::AlreadyTracked);
        }
        if lines.len() >= MAX_CONSUMERS {
            // Relaxed: a monotonic statistic with no ordering against other memory.
            self.untracked.fetch_add(1, Ordering::Relaxed);
            return Err(HealthError::ConsumerCapacity {
                tracked: lines.len(),
            });
        }
        let line = Arc::new(Line {
            client_id,
            consumer,
            flow: Mutex::new(Flow::default()),
        });
        lines.push(Arc::downgrade(&line));
        drop(lines);
        Ok(ConsumerGauge(line))
    }

    /// Copies every live row as of `now`, ordered by consumer; the table lock is held
    /// only to clone at most [`MAX_CONSUMERS`] handles.
    pub(super) fn rows(&self, now: TimestampMs) -> Rows {
        let mut consumers: Vec<ConsumerQueue> = self
            .live()
            .iter()
            .map(|line| {
                let backlog = line.flow().backlog(now);
                ConsumerQueue::new(line.client_id.clone(), line.consumer.clone(), backlog)
            })
            .collect();
        consumers.sort_by(|left, right| left.consumer.cmp(&right.consumer));
        let streams = consumers
            .iter()
            .filter(|row| matches!(row.consumer, ConsumerRef::Stream { .. }))
            .count();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Rows {
            streams: count(streams),
            subscriptions: count(consumers.len().saturating_sub(streams)),
            consumers,
            untracked: Count::new(self.untracked.load(Ordering::Relaxed)),
        }
    }
}
