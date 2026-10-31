//! Output intake and parse turns.
//!
//! A chunk is accepted only when the parse queue has room, the session output budget
//! grants its bytes and, with history, the history writer queues its append; otherwise it
//! is stashed and retried after the next parse turn, budget release or history job.

use std::sync::Arc;

use ovis_terminal_vt::OutputOffset;

use super::Actor;
use crate::streams::scheduler::event::{OutputEvent, publish};
use crate::streams::scheduler::history::HistoryStore;

fn units(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

impl Actor {
    /// Accepts a fresh `chunk` for parsing and fanout, or stashes it until there is room.
    pub(super) fn accept(&mut self, chunk: Arc<[u8]>) {
        if !self.try_accept(&chunk) {
            self.report.output_waits = self.report.output_waits.saturating_add(1);
            self.stash = Some(chunk);
        }
    }

    /// Retries the stashed chunk; it stays stashed while there is still no room.
    pub(super) fn retry(&mut self, chunk: Arc<[u8]>) {
        if !self.try_accept(&chunk) {
            self.stash = Some(chunk);
        }
    }

    fn try_accept(&mut self, chunk: &Arc<[u8]>) -> bool {
        let room = self.shard.pending_bytes().saturating_add(chunk.len());
        if room > self.shard.cap().bytes() {
            return false;
        }
        let Ok(held) = self.output_budget.reserve(units(chunk.len())) else {
            return false;
        };
        if let Some(history) = &self.history {
            let bytes = Arc::clone(chunk);
            let room = Arc::clone(&self.room);
            let queued = history.try_submit(move |store: &mut HistoryStore| {
                let recorded = store.append(&bytes).is_ok();
                room.notify_one();
                recorded
            });
            if queued.is_err() {
                return false;
            }
        }
        if self.shard.push(Arc::clone(chunk)).is_err() {
            return false;
        }
        match &mut self.output_held {
            Some(output) => output.merge(held),
            None => self.output_held = Some(held),
        }
        let offset = self.accepted;
        self.accepted = OutputOffset::new(offset.get().saturating_add(units(chunk.len())));
        let bytes = Arc::clone(chunk);
        publish(&self.fanout, OutputEvent::Chunk { offset, bytes });
        true
    }

    pub(super) fn turn(&mut self) {
        let turn = self.shard.turn();
        if let Some(output) = &mut self.output_held {
            output.shrink(units(turn.fed));
        }
        let report = &mut self.report;
        report.parse_turns = report.parse_turns.saturating_add(1);
        report.parsed_bytes = report.parsed_bytes.saturating_add(units(turn.fed));
        report.max_turn = report.max_turn.max(turn.elapsed);
        let queries = units(turn.outcome.queries.len());
        report.queries = report.queries.saturating_add(queries);
        let offset = self.shard.vt().offset();
        publish(&self.fanout, OutputEvent::Parsed { offset });
    }
}
