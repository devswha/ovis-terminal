//! The terminal actor: control first, then admissions, then output, then one parse turn.
//!
//! Every loop iteration serves the highest-priority ready lane. A parse turn feeds at most
//! one budget of output and then yields, so a burst of output delays a cancel or resize by
//! at most one turn. Input is written by a separate task, so a program that never reads
//! stdin cannot hold the actor.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_vt::{OutputOffset, Shard, Vt};
use tokio::io::AsyncWrite;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio::task::JoinHandle;

use super::config::TerminalSetup;
use super::error::SchedulerError;
use super::event::{OutputEvent, TerminalEvent, publish};
use super::handle::{Admit, Control, OutputFeed, Receipt, TerminalHandle};
use super::input::{CancelOutcome, InputQueue, Ticket};
use super::writer::{Shared, WriterReport, drain};
use crate::streams::scheduler::budget::ByteBudget;
use crate::streams::scheduler::error::BudgetScope;

/// What one actor did before it stopped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActorReport {
    /// Input entries admitted.
    pub admitted: u64,
    /// Queued entries withdrawn by a cancel.
    pub withdrawn: u64,
    /// Control requests served.
    pub controls: u64,
    /// Resizes committed.
    pub resized: u64,
    /// Parse turns run.
    pub parse_turns: u64,
    /// Output bytes parsed.
    pub parsed_bytes: u64,
    /// The longest parse turn.
    pub max_turn: Duration,
    /// Times output waited because the parse queue was full.
    pub output_waits: u64,
    /// Terminal queries the screen completed; their replies belong to the stream hub.
    pub queries: u64,
    /// Entries still queued at stop; none of their bytes were written.
    pub unsent: usize,
    /// The entry cut off mid-write at stop; how much of it arrived is unknown.
    pub cut_off: Option<Ticket>,
    /// The input writer's account, absent when it was cut off mid-write.
    pub writer: Option<WriterReport>,
}

/// The running actor; join it after [`TerminalHandle::stop`].
#[derive(Debug)]
pub struct ActorTask(JoinHandle<ActorReport>);

impl ActorTask {
    /// Waits for the actor and its writer to end.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the actor task panicked or was aborted.
    pub async fn join(self) -> Result<ActorReport, SchedulerError> {
        Ok(self.0.await?)
    }
}

/// Starts the actor of one terminal on the current Tokio runtime.
///
/// `sink` is the program's input; output enters through the returned feed.
pub fn spawn_terminal<W>(
    vt: Vt,
    sink: W,
    setup: &TerminalSetup,
) -> (TerminalHandle, OutputFeed, ActorTask)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let config = setup.config;
    let (controls, control_rx) = mpsc::channel(config.controls.max(1));
    let (admits, admit_rx) = mpsc::channel(config.admits.max(1));
    let (chunks, output_rx) = mpsc::channel(config.output_chunks.max(1));
    let (events, _) = broadcast::channel(config.events.max(1));
    let (output, _) = broadcast::channel(config.output_events.max(1));
    let shared = Arc::new(Shared {
        queue: std::sync::Mutex::new(InputQueue::default()),
        wake: Notify::new(),
    });
    let writer = tokio::spawn(drain(Arc::clone(&shared), sink, events.clone()));
    let actor = Actor {
        shard: Shard::new(vt, config.parse, config.pending),
        controls: control_rx,
        admits: admit_rx,
        output: output_rx,
        output_open: true,
        stash: None,
        accepted: OutputOffset::ZERO,
        shared,
        events: events.clone(),
        fanout: output.clone(),
        report: ActorReport::default(),
    };
    let handle = TerminalHandle {
        controls,
        admits,
        budgets: [
            ByteBudget::new(BudgetScope::TerminalInput, config.terminal_input),
            Arc::clone(&setup.session),
        ],
        events,
        output,
    };
    let feed = OutputFeed {
        chunks,
        max_chunk: config.pending.bytes().max(1),
    };
    (handle, feed, ActorTask(tokio::spawn(actor.run(writer))))
}

struct Actor {
    shard: Shard,
    controls: mpsc::Receiver<Control>,
    admits: mpsc::Receiver<Admit>,
    output: mpsc::Receiver<Arc<[u8]>>,
    output_open: bool,
    stash: Option<Arc<[u8]>>,
    accepted: OutputOffset,
    shared: Arc<Shared>,
    events: broadcast::Sender<TerminalEvent>,
    fanout: broadcast::Sender<OutputEvent>,
    report: ActorReport,
}

impl Actor {
    async fn run(mut self, writer: JoinHandle<WriterReport>) -> ActorReport {
        loop {
            if let Some(chunk) = self.stash.take() {
                self.accept(chunk);
            }
            let parse = self.shard.pending_bytes() > 0;
            let take_output = self.output_open && self.stash.is_none();
            tokio::select! {
                biased;
                control = self.controls.recv() => match control {
                    Some(Control::Stop) | None => break,
                    Some(control) => self.control(control),
                },
                admit = self.admits.recv() => match admit {
                    Some(admit) => self.admit(admit),
                    None => break,
                },
                chunk = self.output.recv(), if take_output => match chunk {
                    Some(chunk) => {
                        if !self.accept(chunk) {
                            self.report.output_waits = self.report.output_waits.saturating_add(1);
                        }
                    }
                    None => self.output_open = false,
                },
                () = std::future::ready(()), if parse => {
                    self.turn();
                    tokio::task::yield_now().await;
                }
            }
        }
        self.stop(writer).await
    }

    fn control(&mut self, control: Control) {
        self.report.controls = self.report.controls.saturating_add(1);
        match control {
            Control::Cancel { ticket, reply } => {
                let outcome = self
                    .shared
                    .queue
                    .lock()
                    .map_or(CancelOutcome::NotQueued, |mut queue| queue.cancel(ticket));
                if outcome == CancelOutcome::Withdrawn {
                    self.report.withdrawn = self.report.withdrawn.saturating_add(1);
                    let at = Instant::now();
                    publish(&self.events, TerminalEvent::Withdrawn { ticket, at });
                }
                reply.send(outcome).ok();
            }
            Control::Resize { geometry, reply } => {
                self.shard.vt_mut().resize(geometry);
                self.report.resized = self.report.resized.saturating_add(1);
                let at = Instant::now();
                publish(&self.events, TerminalEvent::Resized { geometry, at });
                reply.send(()).ok();
            }
            Control::Stop => {}
        }
    }

    fn admit(&mut self, admit: Admit) {
        let Admit { bytes, held, reply } = admit;
        let Ok(mut queue) = self.shared.queue.lock() else {
            return;
        };
        let ticket = queue.admit(bytes, held);
        let queued = queue.queued();
        drop(queue);
        self.shared.wake.notify_one();
        self.report.admitted = self.report.admitted.saturating_add(1);
        let at = Instant::now();
        publish(&self.events, TerminalEvent::Admitted { ticket, at });
        reply.send(Receipt { ticket, queued }).ok();
    }

    /// Queues a chunk for parsing, or keeps it until a turn makes room.
    fn accept(&mut self, chunk: Arc<[u8]>) -> bool {
        if self.shard.push(Arc::clone(&chunk)).is_err() {
            self.stash = Some(chunk);
            return false;
        }
        let offset = self.accepted;
        let len = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        self.accepted = OutputOffset::new(offset.get().saturating_add(len));
        publish(
            &self.fanout,
            OutputEvent::Chunk {
                offset,
                bytes: chunk,
            },
        );
        true
    }

    fn turn(&mut self) {
        let turn = self.shard.turn();
        let report = &mut self.report;
        report.parse_turns = report.parse_turns.saturating_add(1);
        let fed = u64::try_from(turn.fed).unwrap_or(u64::MAX);
        report.parsed_bytes = report.parsed_bytes.saturating_add(fed);
        report.max_turn = report.max_turn.max(turn.elapsed);
        let queries = u64::try_from(turn.outcome.queries.len()).unwrap_or(u64::MAX);
        report.queries = report.queries.saturating_add(queries);
        let offset = self.shard.vt().offset();
        publish(&self.fanout, OutputEvent::Parsed { offset });
    }

    /// Closes the input queue and joins the writer; a writer stuck mid-write is aborted.
    async fn stop(mut self, writer: JoinHandle<WriterReport>) -> ActorReport {
        let (unsent, cut_off) = self
            .shared
            .queue
            .lock()
            .map_or((0, None), |mut queue| queue.close());
        self.report.unsent = unsent;
        self.report.cut_off = cut_off;
        if cut_off.is_some() {
            writer.abort();
        }
        self.shared.wake.notify_one();
        self.report.writer = writer.await.ok();
        self.report
    }
}
