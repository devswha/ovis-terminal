//! The per-terminal hub: one lock orders feeds against registrations.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ovis_terminal_holder::protocol::{Exit, Output};
use ovis_terminal_vt::OutputOffset;

use super::board::Board;
use super::cancel::CancelScope;
use super::entry::{Armed, Chunk, Condition, Entry, OutcomeOf, Start, WatchId};
use super::error::WaitError;
use super::facts::{ExitEvidence, Facts, ProcessState, StatusFact};
use super::interrupt::{Observed, StopWatch};
use super::output::{OutputOutcome, OutputWait};
use super::slot::{Slot, Ticket};
use super::state::{StateOutcome, StateWait};
use super::turn::{TurnBinding, TurnOutcome};
use crate::screen::{Ingested, Screen, ScreenTransition};

#[derive(Debug)]
struct Table {
    board: Board,
    entries: BTreeMap<WatchId, Entry>,
    next: u64,
}

impl Table {
    /// Re-evaluates every observer against the facts and drops the resolved ones.
    fn settle(&mut self) {
        let facts = &self.board.facts;
        self.entries.retain(|_, entry| !entry.settle(facts));
    }

    fn feed(&mut self, chunk: Chunk<'_>) -> Result<(), WaitError> {
        let Self { board, entries, .. } = self;
        let mut resolved = Vec::new();
        for (id, entry) in entries.iter_mut() {
            if entry.feed(chunk, &board.screen)? {
                resolved.push(*id);
            }
        }
        for id in resolved {
            entries.remove(&id);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Shared {
    table: Mutex<Table>,
}

impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, Table>, WaitError> {
        self.table.lock().map_err(|_| WaitError::Poisoned)
    }

    /// Resolves `id` as timed out, unless a feed resolved it first.
    pub(super) fn expire(&self, id: WatchId) -> Result<(), WaitError> {
        let mut table = self.lock()?;
        if let Some(entry) = table.entries.remove(&id) {
            entry.expire(&table.board.facts);
        }
        drop(table);
        Ok(())
    }

    /// Resolves `id` as cancelled, unless it already resolved.
    pub(super) fn cancel(&self, id: WatchId) {
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = table.entries.remove(&id) {
            entry.cancel(&table.board.facts);
        }
    }

    /// Drops an observer whose ticket went away; removing an entry never tears the table.
    pub(super) fn detach(&self, id: WatchId) {
        self.table
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .remove(&id);
    }
}

/// Waits of one terminal incarnation, fed by its holder output, exit and declared status.
#[derive(Debug, Clone)]
pub struct Waits {
    shared: Arc<Shared>,
}

impl Waits {
    /// The hub of the incarnation `screen` belongs to; its process is running.
    pub fn new(screen: Screen) -> Self {
        Self {
            shared: Arc::new(Shared {
                table: Mutex::new(Table {
                    board: Board {
                        screen,
                        facts: Facts::new(),
                    },
                    entries: BTreeMap::new(),
                    next: 0,
                }),
            }),
        }
    }

    /// Offset after the newest output byte: a cursor a later output wait starts from.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Poisoned`] or the screen's refusal.
    pub fn cursor(&self) -> Result<OutputOffset, WaitError> {
        let table = self.shared.lock()?;
        Ok(table.board.screen.live().map_err(WaitError::Screen)?.offset)
    }

    /// Liveness of the process, from holder evidence.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Poisoned`] when the table is poisoned.
    pub fn process(&self) -> Result<ProcessState, WaitError> {
        Ok(self.shared.lock()?.board.facts.process)
    }

    /// Arms an output wait; retained output since its cursor is matched before arming.
    ///
    /// # Errors
    ///
    /// Returns `output_gap` for a cursor behind retention, `invalid_params` for one ahead.
    pub fn output(
        &self,
        wait: OutputWait,
        scope: &CancelScope,
    ) -> Result<Ticket<OutputOutcome>, WaitError> {
        self.arm(wait, scope)
    }

    /// Arms a state wait; one already satisfied answers `already_satisfied` at once.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Poisoned`] when the table is poisoned.
    pub fn state(
        &self,
        wait: StateWait,
        scope: &CancelScope,
    ) -> Result<Ticket<StateOutcome>, WaitError> {
        self.arm(wait, scope)
    }

    /// Arms a turn wait bound to one send or one turn.
    ///
    /// # Errors
    ///
    /// Returns `unknown_turn` for a turn number that passed without being retained.
    pub fn turn(
        &self,
        binding: TurnBinding,
        scope: &CancelScope,
    ) -> Result<Ticket<TurnOutcome>, WaitError> {
        self.arm(binding, scope)
    }

    /// Arms stop observation; arm it before the interrupt key is admitted.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Poisoned`] when the table is poisoned.
    pub fn stop(
        &self,
        watch: StopWatch,
        scope: &CancelScope,
    ) -> Result<Ticket<Observed>, WaitError> {
        self.arm(watch, scope)
    }

    /// Evaluates `condition` and, unless it already resolved, arms it in the same step.
    fn arm<C: Condition>(
        &self,
        condition: C,
        scope: &CancelScope,
    ) -> Result<Ticket<OutcomeOf<C>>, WaitError> {
        let slot = Slot::new();
        let mut table = self.shared.lock()?;
        if scope.is_cancelled() {
            slot.fill(condition.cancelled(&table.board.facts));
            return Ok(Ticket::new(Arc::clone(&self.shared), None, slot));
        }
        let id = match condition.start(&table.board)? {
            Start::Done(outcome) => {
                slot.fill(outcome);
                None
            }
            Start::Armed(watch) => {
                let id = WatchId(table.next);
                table.next = table.next.saturating_add(1);
                let slot = Arc::clone(&slot);
                table.entries.insert(id, C::entry(Armed { watch, slot }));
                Some(id)
            }
        };
        drop(table);
        if let Some(id) = id {
            scope.enlist(&self.shared, id);
        }
        Ok(Ticket::new(Arc::clone(&self.shared), id, slot))
    }

    /// Feeds one holder output chunk to the screen and to every armed output wait.
    ///
    /// # Errors
    ///
    /// Returns the screen's refusal; a refused chunk reaches no wait.
    pub fn ingest(&self, output: &Output) -> Result<Ingested, WaitError> {
        let mut table = self.shared.lock()?;
        let ingested = table
            .board
            .screen
            .ingest(output)
            .map_err(WaitError::Screen)?;
        table.feed(Chunk {
            from: OutputOffset::new(output.offset),
            bytes: &output.bytes,
        })?;
        drop(table);
        Ok(ingested)
    }

    /// Records the holder's exit report after the final output byte.
    ///
    /// # Errors
    ///
    /// Returns the screen's refusal, such as a target mismatch or missing output.
    pub fn finish(&self, exit: &Exit) -> Result<Vec<ScreenTransition>, WaitError> {
        let mut table = self.shared.lock()?;
        let transitions = table.board.screen.finish(exit).map_err(WaitError::Screen)?;
        table.board.facts.process = ProcessState::Exited(ExitEvidence::of(exit));
        table.settle();
        drop(table);
        Ok(transitions)
    }

    /// Records that the holder link ended without an exit report.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Poisoned`] when the table is poisoned.
    pub fn lost(&self) -> Result<(), WaitError> {
        let mut table = self.shared.lock()?;
        if table.board.facts.process == ProcessState::Running {
            table.board.facts.process = ProcessState::Lost;
        }
        table.settle();
        drop(table);
        Ok(())
    }

    /// Applies one accepted declared status report.
    ///
    /// # Errors
    ///
    /// Rejects a report that does not advance the sequence or the turn, or arrives after exit.
    pub fn report(&self, fact: StatusFact) -> Result<(), WaitError> {
        let mut table = self.shared.lock()?;
        table.board.facts.report(fact)?;
        table.settle();
        drop(table);
        Ok(())
    }
}
