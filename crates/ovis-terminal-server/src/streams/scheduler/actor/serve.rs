//! Control requests, admissions and shutdown.

use std::time::Instant;

use tokio::task::JoinHandle;

use super::{Actor, ActorReport};
use crate::streams::scheduler::error::SchedulerError;
use crate::streams::scheduler::event::{TerminalEvent, publish};
use crate::streams::scheduler::handle::{Admit, Control, Queued, Receipt};
use crate::streams::scheduler::history::HistoryStore;
use crate::streams::scheduler::input::CancelOutcome;
use crate::streams::scheduler::writer::WriterReport;

impl Actor {
    pub(super) fn control(&mut self, queued: Queued) {
        let Queued { control, slot } = queued;
        self.report.controls = self.report.controls.saturating_add(1);
        let after_turns = self.report.parse_turns;
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
                    let event = TerminalEvent::Withdrawn {
                        ticket,
                        at,
                        after_turns,
                    };
                    publish(&self.events, event);
                }
                reply.send(outcome).ok();
            }
            Control::Resize { geometry, reply } => {
                self.shard.vt_mut().resize(geometry);
                self.report.resized = self.report.resized.saturating_add(1);
                let at = Instant::now();
                let event = TerminalEvent::Resized {
                    geometry,
                    at,
                    after_turns,
                };
                publish(&self.events, event);
                reply.send(()).ok();
            }
            Control::ReadHistory { offset, max, reply } => {
                let read = self.history.as_ref().map_or_else(
                    || Err(SchedulerError::NoHistory),
                    |history| {
                        history.try_submit(move |store: &mut HistoryStore| store.read(offset, max))
                    },
                );
                reply.send(read).ok();
            }
            Control::Stop => {}
        }
        drop(slot);
    }

    pub(super) fn admit(&mut self, admit: Admit) {
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
        let after_turns = self.report.parse_turns;
        let event = TerminalEvent::Admitted {
            ticket,
            at,
            after_turns,
        };
        publish(&self.events, event);
        reply.send(Receipt { ticket, queued }).ok();
    }

    /// Closes the input queue and joins the writer (aborting one stuck mid-write), then
    /// returns the session reservations and joins the history thread.
    pub(super) async fn stop(mut self, writer: JoinHandle<WriterReport>) -> ActorReport {
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
        drop(self.retention);
        drop(self.output_held);
        if let Some(history) = self.history {
            let joined = tokio::task::spawn_blocking(move || history.shutdown()).await;
            self.report.history = joined
                .ok()
                .and_then(Result::ok)
                .and_then(|store| store.stats().ok());
        }
        self.report
    }
}
