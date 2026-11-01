//! Control-event clients: committed events travel to a real socket per client.
//!
//! Each client owns a bounded queue, one forwarder per terminal subscription and one
//! writer; no lock is held across a write (V-4085) and a forwarder never waits on a full
//! queue, so a client that stops reading loses its own events, counted, and slows no
//! other client (IDEAL-041). A sample is commit-to-written: the line was accepted by the
//! client's writable socket.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ovis_terminal_server::streams::scheduler::TerminalEvent;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::lanes::Progress;

const QUEUE: usize = 4096;

/// One committed event on its way to a client.
#[derive(Debug)]
struct Committed {
    at: Instant,
    line: Vec<u8>,
}

/// Counters a client's forwarders share.
#[derive(Debug, Default)]
struct Shared {
    lost: AtomicU64,
    deepest: AtomicUsize,
}

/// What a writer did.
#[derive(Debug, Default)]
pub(super) struct Written {
    pub(super) warmup: Vec<Duration>,
    pub(super) measured: Vec<Duration>,
    pub(super) undelivered: u64,
    pub(super) write_failed: bool,
}

/// One client's account after the run.
#[derive(Debug)]
pub(super) struct ClientOutcome {
    pub(super) index: usize,
    pub(super) reads: bool,
    pub(super) written: Written,
    pub(super) lost: u64,
    pub(super) deepest_queue: usize,
    pub(super) bytes_read: u64,
}

/// A running client.
#[derive(Debug)]
pub(super) struct Client {
    index: usize,
    reads: bool,
    shared: Arc<Shared>,
    forwarders: Vec<JoinHandle<()>>,
    writer: JoinHandle<Written>,
    reader: Option<JoinHandle<u64>>,
    unread: Option<UnixStream>,
    abandon: watch::Sender<bool>,
}

/// How one client behaves.
#[derive(Debug, Clone)]
pub(super) struct Spec {
    pub(super) index: usize,
    pub(super) reads: bool,
    pub(super) delay: Option<Duration>,
    pub(super) progress: Arc<Progress>,
}

fn line(terminal: usize, event: &TerminalEvent) -> Option<Committed> {
    let (kind, at) = match event {
        TerminalEvent::Admitted { at, .. } => ("admitted", *at),
        TerminalEvent::Withdrawn { at, .. } => ("withdrawn", *at),
        TerminalEvent::Resized { at, .. } => ("resized", *at),
        _ => return None,
    };
    let line = format!("{{\"event\":\"{kind}\",\"terminal\":\"{terminal}\"}}\n").into_bytes();
    Some(Committed { at, line })
}

async fn forward(
    (terminal, mut events): (usize, Receiver<TerminalEvent>),
    queue: mpsc::Sender<Committed>,
    shared: Arc<Shared>,
) {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(missed)) => {
                shared.lost.fetch_add(missed, Ordering::AcqRel);
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        let Some(committed) = line(terminal, &event) else {
            continue;
        };
        if queue.try_send(committed).is_err() {
            shared.lost.fetch_add(1, Ordering::AcqRel);
        }
        let depth = queue.max_capacity().saturating_sub(queue.capacity());
        shared.deepest.fetch_max(depth, Ordering::AcqRel);
    }
}

async fn write(
    (spec, mut socket): (Spec, UnixStream),
    mut queue: mpsc::Receiver<Committed>,
    mut abandon: watch::Receiver<bool>,
) -> Written {
    let mut written = Written::default();
    while let Some(committed) = queue.recv().await {
        let delivery = async {
            if let Some(delay) = spec.delay {
                tokio::time::sleep(delay).await;
            }
            socket.write_all(&committed.line).await
        };
        let outcome = tokio::select! {
            biased;
            _ = abandon.changed() => None,
            result = delivery => Some(result.is_ok()),
        };
        match outcome {
            Some(true) => {
                let sample = committed.at.elapsed();
                match spec.progress.measuring_from() {
                    Some(from) if committed.at >= from => written.measured.push(sample),
                    _ => written.warmup.push(sample),
                }
            }
            Some(false) => {
                written.write_failed = true;
                written.undelivered = written.undelivered.saturating_add(1);
                break;
            }
            None => {
                written.undelivered = written.undelivered.saturating_add(1);
                break;
            }
        }
    }
    queue.close();
    while queue.recv().await.is_some() {
        written.undelivered = written.undelivered.saturating_add(1);
    }
    written
}

async fn drain(mut peer: UnixStream) -> u64 {
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    while let Ok(read) = peer.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    total
}

impl Client {
    /// Starts the client on every subscription in `events`, indexed by terminal.
    pub(super) fn start(spec: Spec, events: Vec<Receiver<TerminalEvent>>) -> std::io::Result<Self> {
        let (socket, peer) = UnixStream::pair()?;
        let (queue, pending) = mpsc::channel(QUEUE);
        let (abandon, abandoned) = watch::channel(false);
        let shared = Arc::new(Shared::default());
        let forwarders = events
            .into_iter()
            .enumerate()
            .map(|subscription| {
                tokio::spawn(forward(subscription, queue.clone(), Arc::clone(&shared)))
            })
            .collect();
        let (index, reads) = (spec.index, spec.reads);
        let writer = tokio::spawn(write((spec, socket), pending, abandoned));
        let (reader, unread) = if reads {
            (Some(tokio::spawn(drain(peer))), None)
        } else {
            (None, Some(peer))
        };
        Ok(Self {
            index,
            reads,
            shared,
            forwarders,
            writer,
            reader,
            unread,
            abandon,
        })
    }

    pub(super) const fn index(&self) -> usize {
        self.index
    }

    /// Hangs up a non-reader's unread end, so its blocked writer fails and stops.
    pub(super) fn hang_up_unread(&mut self) {
        self.unread = None;
    }

    /// Awaits the forwarders (their subscriptions must be closed), then lets the writer
    /// drain for at most `drain`; whatever is still queued then is undelivered.
    pub(super) async fn finish(self, drain: Duration) -> std::io::Result<ClientOutcome> {
        for forwarder in self.forwarders {
            forwarder.await.map_err(std::io::Error::other)?;
        }
        let mut writer = self.writer;
        let written = match tokio::time::timeout(drain, &mut writer).await {
            Ok(joined) => joined,
            Err(_elapsed) => {
                self.abandon.send_replace(true);
                writer.await
            }
        }
        .map_err(std::io::Error::other)?;
        drop(self.unread);
        let bytes_read = match self.reader {
            Some(reader) => reader.await.map_err(std::io::Error::other)?,
            None => 0,
        };
        Ok(ClientOutcome {
            index: self.index,
            reads: self.reads,
            written,
            lost: self.shared.lost.load(Ordering::Acquire),
            deepest_queue: self.shared.deepest.load(Ordering::Acquire),
            bytes_read,
        })
    }
}
