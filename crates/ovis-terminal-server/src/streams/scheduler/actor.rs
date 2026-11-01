//! The terminal actor: control first, then admissions, then output, then one parse turn.
//!
//! Every loop iteration serves the highest-priority ready lane. A parse turn feeds at most
//! one budget of output and then yields, so a control request queued behind a burst of
//! output waits for at most one parse turn of its actor. Control events carry the number
//! of parse turns run before they committed, which is how tests pin this ordering. Input
//! is written by a separate task, so a program that never reads stdin cannot hold the
//! actor; output waits for room in the parse queue, the session output budget and the
//! history writer queue before it is accepted.

mod intake;
mod serve;

use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_vt::{OutputOffset, Shard};
use tokio::io::AsyncWrite;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio::task::JoinHandle;

use super::budget::{Budget, Reservation};
use super::config::{Engine, TerminalSetup};
use super::db::DbWriter;
use super::error::{BudgetScope, SchedulerError};
use super::event::{OutputEvent, TerminalEvent};
use super::handle::{Admit, Control, OutputFeed, Queued, TerminalHandle};
use super::history::{HistoryStats, HistoryStore};
use super::input::{InputQueue, Ticket};
use super::writer::{Shared, WriterReport, drain};

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
    /// The longest parse turn, wall time.
    pub max_turn: Duration,
    /// Times output waited for room in the parse queue, session budget or history queue.
    pub output_waits: u64,
    /// Terminal queries the screen completed; their replies belong to the stream hub.
    pub queries: u64,
    /// Entries still queued at stop; none of their bytes were written.
    pub unsent: usize,
    /// The entry cut off mid-write at stop; how much of it arrived is unknown.
    pub cut_off: Option<Ticket>,
    /// The input writer's account, absent when it was cut off mid-write.
    pub writer: Option<WriterReport>,
    /// The disk history at stop, when the terminal kept one.
    pub history: Option<HistoryStats>,
}

/// The running actor; join it after [`TerminalHandle::stop`].
#[derive(Debug)]
pub struct ActorTask(JoinHandle<ActorReport>);

impl ActorTask {
    /// Waits for the actor, its writer and its history thread to end.
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
///
/// # Errors
///
/// Returns `backpressure` when the session event budget cannot hold this terminal's
/// event retention, and `internal` when the history thread cannot start.
pub fn spawn_terminal<W>(
    engine: Engine,
    sink: W,
    setup: &TerminalSetup,
) -> Result<(TerminalHandle, OutputFeed, ActorTask), SchedulerError>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let config = setup.config;
    let retention = setup.session.events.reserve(config.retention_bytes())?;
    let history = engine
        .history
        .map(|store| DbWriter::spawn("terminal-history", store, config.history_jobs))
        .transpose()?;
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
        shard: Shard::new(engine.vt, config.parse, config.pending),
        controls: control_rx,
        admits: admit_rx,
        output: output_rx,
        output_open: true,
        stash: None,
        accepted: OutputOffset::ZERO,
        shared,
        events: events.clone(),
        fanout: output.clone(),
        output_budget: Arc::clone(&setup.session.output),
        output_held: None,
        history,
        room: Arc::new(Notify::new()),
        retention,
        report: ActorReport::default(),
    };
    let handle = TerminalHandle {
        controls,
        admits,
        budgets: [
            Budget::new(BudgetScope::TerminalInput, config.terminal_input),
            Arc::clone(&setup.session.input),
        ],
        control_slots: Arc::clone(&setup.session.control),
        events,
        output,
    };
    let feed = OutputFeed {
        chunks,
        max_chunk: config.chunk_bytes.max(1),
    };
    Ok((handle, feed, ActorTask(tokio::spawn(actor.run(writer)))))
}

struct Actor {
    shard: Shard,
    controls: mpsc::Receiver<Queued>,
    admits: mpsc::Receiver<Admit>,
    output: mpsc::Receiver<Arc<[u8]>>,
    output_open: bool,
    stash: Option<Arc<[u8]>>,
    accepted: OutputOffset,
    shared: Arc<Shared>,
    events: broadcast::Sender<TerminalEvent>,
    fanout: broadcast::Sender<OutputEvent>,
    output_budget: Arc<Budget>,
    output_held: Option<Reservation>,
    history: Option<DbWriter<HistoryStore>>,
    room: Arc<Notify>,
    retention: Reservation,
    report: ActorReport,
}

impl Actor {
    async fn run(mut self, writer: JoinHandle<WriterReport>) -> ActorReport {
        let budget = Arc::clone(&self.output_budget);
        let room = Arc::clone(&self.room);
        loop {
            let released = budget.released();
            if let Some(chunk) = self.stash.take() {
                self.retry(chunk);
            }
            let parse = self.shard.pending_bytes() > 0;
            let waiting = self.stash.is_some();
            let take_output = self.output_open && !waiting;
            tokio::select! {
                biased;
                queued = self.controls.recv() => match queued {
                    Some(Queued { control: Control::Stop, .. }) | None => break,
                    Some(queued) => self.control(queued),
                },
                admit = self.admits.recv() => match admit {
                    Some(admit) => self.admit(admit),
                    None => break,
                },
                chunk = self.output.recv(), if take_output => match chunk {
                    Some(chunk) => self.accept(chunk),
                    None => self.output_open = false,
                },
                () = std::future::ready(()), if parse => {
                    self.turn();
                    tokio::task::yield_now().await;
                }
                () = released, if waiting && !parse => {}
                () = room.notified(), if waiting && !parse => {}
            }
        }
        self.stop(writer).await
    }
}
