//! One connected viewer: a bounded queue drained by its own writer thread, so a viewer
//! that stops reading never blocks the control loop or any other viewer.

use std::io::{ErrorKind, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use ovis_terminal_protocol::schema::scalar::{ByteCount, Count};

use super::super::super::error::SoakError;
use super::super::super::wire::{Hello, ViewerHealth};

/// Records one queue may hold whatever their size; bytes are the real bound.
const RECORDS: usize = 1 << 16;

#[derive(Debug)]
pub(super) struct Viewer {
    pub(super) hello: Hello,
    queue: Option<SyncSender<Arc<str>>>,
    queued: Arc<AtomicU64>,
    max_queued: u64,
    sent: u64,
    pub(super) dropped: u64,
    /// Shutdowns that failed for another reason than an already gone peer, and
    /// writer threads that did not end cleanly.
    failures: u64,
    stream: UnixStream,
    writer: Option<JoinHandle<()>>,
}

fn drain(lines: &Receiver<Arc<str>>, (mut stream, queued): (UnixStream, &AtomicU64)) {
    for line in lines {
        if stream.write_all(line.as_bytes()).is_err() {
            return;
        }
        let length = u64::try_from(line.len()).unwrap_or(u64::MAX);
        queued.fetch_sub(length, Ordering::AcqRel);
    }
}

impl Viewer {
    pub(super) fn start(hello: Hello, stream: UnixStream) -> Result<Self, SoakError> {
        let (queue, lines) = std::sync::mpsc::sync_channel(RECORDS);
        let queued = Arc::new(AtomicU64::new(0));
        let writing = stream
            .try_clone()
            .map_err(SoakError::io("clone a viewer stream"))?;
        let counter = Arc::clone(&queued);
        let writer = std::thread::Builder::new()
            .name("soak-viewer".to_owned())
            .spawn(move || drain(&lines, (writing, &counter)))
            .map_err(SoakError::io("start a viewer writer"))?;
        Ok(Self {
            hello,
            queue: Some(queue),
            queued,
            max_queued: 0,
            sent: 0,
            dropped: 0,
            failures: 0,
            stream,
            writer: Some(writer),
        })
    }

    pub(super) const fn cut(&self) -> bool {
        self.queue.is_none()
    }

    pub(super) fn queued(&self) -> u64 {
        self.queued.load(Ordering::Acquire)
    }

    /// Queues `line`; `false` when it would exceed `limit` bytes and the viewer was cut.
    pub(super) fn offer(&mut self, line: &Arc<str>, limit: u64) -> bool {
        let Some(queue) = &self.queue else {
            return true;
        };
        let length = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let after = self.queued().saturating_add(length);
        if after > limit {
            self.disconnect();
            return false;
        }
        self.queued.fetch_add(length, Ordering::AcqRel);
        match queue.try_send(Arc::clone(line)) {
            Ok(()) => {
                self.max_queued = self.max_queued.max(after);
                self.sent = self.sent.saturating_add(1);
                true
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.queued.fetch_sub(length, Ordering::AcqRel);
                self.disconnect();
                false
            }
        }
    }

    /// Stops queueing and ends the connection; the writer thread then finishes.
    pub(super) fn disconnect(&mut self) {
        if self.queue.take().is_some() {
            self.dropped = self.dropped.saturating_add(1);
        }
        match self.stream.shutdown(Shutdown::Both) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotConnected => {}
            Err(_) => self.failures = self.failures.saturating_add(1),
        }
    }

    pub(super) fn health(&self) -> ViewerHealth {
        ViewerHealth {
            viewer: self.hello.viewer.clone(),
            reads: self.hello.reads,
            queued_bytes: ByteCount::new(self.queued()),
            max_queued_bytes: ByteCount::new(self.max_queued),
            sent: Count::new(self.sent),
            cut: self.cut(),
            failures: Count::new(self.failures),
        }
    }

    /// Disconnects and joins the writer.
    pub(super) fn close(mut self) {
        self.disconnect();
        if let Some(writer) = self.writer.take()
            && writer.join().is_err()
        {
            self.failures = self.failures.saturating_add(1);
        }
    }
}
