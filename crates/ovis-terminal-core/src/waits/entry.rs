//! Armed observers and the contract each wait kind fulfils.

use std::sync::Arc;

use ovis_terminal_vt::OutputOffset;

use super::board::Board;
use super::error::WaitError;
use super::facts::Facts;
use super::interrupt::Baseline;
use super::output::OutputWatch;
use super::slot::Slot;
use super::state::StateWait;
use super::turn::TurnBinding;
use crate::screen::Screen;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct WatchId(pub u64);

#[derive(Debug, Clone, Copy)]
pub(super) struct Chunk<'a> {
    pub from: OutputOffset,
    pub bytes: &'a [u8],
}

/// What an armed observer does when the facts change, its deadline passes or it is cancelled.
pub(super) trait Watch {
    type Outcome;
    fn settle(&mut self, facts: &Facts) -> Option<Self::Outcome>;
    fn expire(&self, facts: &Facts) -> Self::Outcome;
    fn cancelled(&self, facts: &Facts) -> Self::Outcome;
}

/// A registration's answer while the table is locked: resolved now, or armed.
#[derive(Debug)]
pub(super) enum Start<W: Watch> {
    Done(W::Outcome),
    Armed(W),
}

impl<W: Watch> Start<W> {
    /// Resolved when the facts already decide the wait, armed otherwise.
    pub(super) fn settled(mut watch: W, facts: &Facts) -> Self {
        watch.settle(facts).map_or(Self::Armed(watch), Self::Done)
    }
}

/// A wait request: evaluated against the current facts before anything is armed.
pub(super) trait Condition {
    type Watch: Watch;
    fn start(self, board: &Board) -> Result<Start<Self::Watch>, WaitError>;
    fn cancelled(&self, facts: &Facts) -> <Self::Watch as Watch>::Outcome;
    fn entry(armed: Armed<Self::Watch>) -> Entry;
}

#[derive(Debug)]
pub(super) struct Armed<W: Watch> {
    pub watch: W,
    pub slot: Arc<Slot<W::Outcome>>,
}

impl<W: Watch> Armed<W> {
    fn settle(&mut self, facts: &Facts) -> bool {
        self.watch
            .settle(facts)
            .map(|outcome| self.slot.fill(outcome))
            .is_some()
    }

    fn expire(self, facts: &Facts) {
        self.slot.fill(self.watch.expire(facts));
    }

    fn cancel(self, facts: &Facts) {
        self.slot.fill(self.watch.cancelled(facts));
    }
}

#[derive(Debug)]
pub(super) enum Entry {
    Output(Armed<OutputWatch>),
    State(Armed<StateWait>),
    Turn(Armed<TurnBinding>),
    Stop(Armed<Baseline>),
}

impl Entry {
    /// Re-evaluates against changed facts; true when the observer resolved.
    pub(super) fn settle(&mut self, facts: &Facts) -> bool {
        match self {
            Self::Output(armed) => armed.settle(facts),
            Self::State(armed) => armed.settle(facts),
            Self::Turn(armed) => armed.settle(facts),
            Self::Stop(armed) => armed.settle(facts),
        }
    }

    /// Scans new output; true when the observer resolved.
    pub(super) fn feed(&mut self, chunk: Chunk<'_>, screen: &Screen) -> Result<bool, WaitError> {
        match self {
            Self::Output(armed) => armed.feed(chunk, screen),
            Self::State(_) | Self::Turn(_) | Self::Stop(_) => Ok(false),
        }
    }

    pub(super) fn expire(self, facts: &Facts) {
        match self {
            Self::Output(armed) => armed.expire(facts),
            Self::State(armed) => armed.expire(facts),
            Self::Turn(armed) => armed.expire(facts),
            Self::Stop(armed) => armed.expire(facts),
        }
    }

    pub(super) fn cancel(self, facts: &Facts) {
        match self {
            Self::Output(armed) => armed.cancel(facts),
            Self::State(armed) => armed.cancel(facts),
            Self::Turn(armed) => armed.cancel(facts),
            Self::Stop(armed) => armed.cancel(facts),
        }
    }
}

impl Armed<OutputWatch> {
    fn feed(&mut self, chunk: Chunk<'_>, screen: &Screen) -> Result<bool, WaitError> {
        Ok(self
            .watch
            .feed(chunk, screen)?
            .map(|outcome| self.slot.fill(outcome))
            .is_some())
    }
}

pub(super) type OutcomeOf<C> = <<C as Condition>::Watch as Watch>::Outcome;
