//! One viewer's delivery: control records first, then credit-limited chunks.
//!
//! Queued control records (checkpoint, gap, readiness, close) never need credit.
//! Paint bytes and live log bytes are sent in chunks within the viewer's own credit,
//! so a viewer that stops acknowledging only stops itself. A deferred close is sent
//! once every byte up to its final log offset has been delivered.

use std::collections::VecDeque;
use std::sync::Arc;

use ovis_terminal_core::leases::Owner;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::records::{AttachControl, ControlKind, StreamClose};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::credit::{Credit, CreditRefusal};
use super::log::{OutputLog, Shared};
use super::offset::{LogOffset, StreamOffset};

#[derive(Debug)]
enum Pending {
    Control(Box<AttachControl>),
    Paint(Shared),
}

/// The next thing a viewer's connection must carry.
#[derive(Debug)]
pub(super) enum Delivery {
    /// A control record; `last` closes the stream after it is sent.
    Control {
        record: Box<AttachControl>,
        last: bool,
    },
    /// Terminal bytes starting at `offset`.
    Data { offset: StreamOffset, bytes: Shared },
}

#[derive(Debug)]
pub(super) struct Viewer {
    pub id: StreamId,
    pub owner: Owner,
    pub interact: bool,
    pub wake: Arc<Notify>,
    credit: Credit,
    queue: VecDeque<Pending>,
    cursor: LogOffset,
    closing: Option<(LogOffset, StreamClose)>,
    closed: bool,
    progress: Instant,
}

impl Viewer {
    pub(super) fn new(id: StreamId, owner: Owner, window: u64) -> Self {
        Self {
            id,
            owner,
            interact: false,
            wake: Arc::new(Notify::new()),
            credit: Credit::new(window),
            queue: VecDeque::new(),
            cursor: LogOffset::ZERO,
            closing: None,
            closed: false,
            progress: Instant::now(),
        }
    }

    /// Offset of the next byte this viewer will be sent, after queued paints.
    pub(super) fn next_offset(&self) -> StreamOffset {
        self.queue
            .iter()
            .fold(self.credit.sent(), |at, pending| match pending {
                Pending::Paint(bytes) => at.advanced(bytes.len()),
                Pending::Control(_) => at,
            })
    }

    pub(super) fn control(&mut self, record: AttachControl) {
        self.queue.push_back(Pending::Control(Box::new(record)));
    }

    /// Queues a checkpoint paint and resumes live bytes at `live`.
    pub(super) fn paint(&mut self, record: AttachControl, (bytes, live): (Arc<[u8]>, LogOffset)) {
        let len = bytes.len();
        self.control(record);
        self.queue
            .push_back(Pending::Paint(Shared::new(bytes, 0..len)));
        self.cursor = live;
    }

    /// Drops undelivered bytes but keeps undelivered control records in order.
    pub(super) fn drop_backlog(&mut self) {
        self.queue
            .retain(|pending| matches!(pending, Pending::Control(_)));
    }

    /// Bytes referenced but not yet sent.
    pub(super) fn backlog(&self, log: &OutputLog) -> u64 {
        let painted = self.next_offset().since(self.credit.sent());
        painted.saturating_add(log.end().since(self.cursor))
    }

    pub(super) const fn cursor(&self) -> LogOffset {
        self.cursor
    }

    pub(super) const fn is_closed(&self) -> bool {
        self.closed || self.closing.is_some()
    }

    /// Closes once the live bytes before `last` are delivered.
    pub(super) fn close_after(&mut self, last: LogOffset, close: StreamClose) {
        if !self.is_closed() {
            self.closing = Some((last, close));
        }
    }

    /// Closes ahead of any undelivered byte; control records already queued go first.
    pub(super) fn close_now(&mut self, close: StreamClose) {
        if !self.is_closed() {
            self.force_close(close);
        }
    }

    /// Like [`Viewer::close_now`], replacing a deferred close that cannot complete.
    pub(super) fn force_close(&mut self, close: StreamClose) {
        if self.closed {
            return;
        }
        self.drop_backlog();
        self.closing = Some((self.cursor, close));
        self.cursor = LogOffset::new(u64::MAX);
    }

    /// The close record was delivered; nothing follows it.
    pub(super) const fn finished(&self) -> bool {
        self.closed
    }

    pub(super) fn acknowledge(&mut self, at: StreamOffset) -> Result<u64, CreditRefusal> {
        if self.closed {
            return Err(CreditRefusal::Closed);
        }
        let available = self.credit.acknowledge(at)?;
        self.progress = Instant::now();
        Ok(available)
    }

    pub(super) fn stalled(&self, log: &OutputLog, after: std::time::Duration) -> bool {
        let pending = self.backlog(log) > 0 || !self.queue.is_empty();
        !self.closed && pending && self.progress.elapsed() >= after
    }

    /// Time left before the stall bound passes without progress.
    pub(super) fn stall_left(&self, after: std::time::Duration) -> std::time::Duration {
        after.saturating_sub(self.progress.elapsed())
    }

    pub(super) fn next(&mut self, log: &OutputLog, chunk: usize) -> Option<Delivery> {
        if self.closed {
            return None;
        }
        let allowance = self.allowance();
        let delivery = match self.queue.front_mut() {
            Some(Pending::Control(_)) => match self.queue.pop_front() {
                Some(Pending::Control(record)) => Some(Delivery::Control {
                    record,
                    last: false,
                }),
                _ => None,
            },
            Some(Pending::Paint(bytes)) => {
                let len = bytes.len().min(chunk).min(allowance);
                let sent = take(bytes, len);
                if bytes.len() == 0 {
                    self.queue.pop_front();
                }
                sent.map(|bytes| self.data(bytes))
            }
            None => self.live(log, chunk),
        };
        if delivery.is_some() {
            self.progress = Instant::now();
        }
        delivery
    }

    fn allowance(&self) -> usize {
        usize::try_from(self.credit.available()).unwrap_or(usize::MAX)
    }

    fn data(&mut self, bytes: Shared) -> Delivery {
        let offset = self.credit.sent();
        self.credit.record(bytes.len());
        Delivery::Data { offset, bytes }
    }

    fn live(&mut self, log: &OutputLog, chunk: usize) -> Option<Delivery> {
        if let Some((last, _)) = &self.closing
            && self.cursor.min(log.end()) >= *last
        {
            return self.finish();
        }
        let until = self
            .closing
            .as_ref()
            .map_or_else(|| log.end(), |(last, _)| *last);
        let room = usize::try_from(until.since(self.cursor)).unwrap_or(usize::MAX);
        let bytes = log.read(self.cursor, chunk.min(self.allowance()).min(room))?;
        self.cursor = self.cursor.advanced(bytes.len());
        Some(self.data(bytes))
    }

    fn finish(&mut self) -> Option<Delivery> {
        let (_, mut close) = self.closing.take()?;
        self.closed = true;
        close.final_offset = self.credit.sent().wire();
        let mut record =
            AttachControl::new(self.id.clone(), ControlKind::Close, close.final_offset);
        record.close = Some(close);
        Some(Delivery::Control {
            record: Box::new(record),
            last: true,
        })
    }
}

fn take(bytes: &mut Shared, len: usize) -> Option<Shared> {
    (len > 0).then(|| bytes.split_front(len))
}
