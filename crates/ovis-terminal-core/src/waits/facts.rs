//! What the hub knows about its process and declared status.

use std::collections::VecDeque;

use ovis_terminal_holder::protocol::Exit;
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::{ReportSeq, TurnNumber};
use ovis_terminal_vt::OutputOffset;
use serde::{Deserialize, Serialize};

use super::error::WaitError;
use crate::input::receipts::InputSeq;

/// Turns kept for waits bound to a send or a turn number.
const TURN_HISTORY: usize = 64;

/// The holder's exit report for the hub's process incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExitEvidence {
    /// Exit code, when the process exited normally.
    pub code: Option<i32>,
    /// Signal number, when a signal ended the process.
    pub signal: Option<i32>,
    /// Offset after the last output byte.
    pub final_offset: OutputOffset,
}

impl ExitEvidence {
    pub(super) const fn of(exit: &Exit) -> Self {
        Self {
            code: exit.code,
            signal: exit.signal,
            final_offset: OutputOffset::new(exit.final_offset),
        }
    }
}

/// Liveness of the hub's process incarnation, from holder evidence only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProcessState {
    /// Spawned, and the holder reported no exit.
    Running,
    /// The holder reported the exit.
    Exited(ExitEvidence),
    /// The holder link ended without an exit report.
    Lost,
}

/// One accepted declared status report (OSC 7501 or the owning process token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StatusFact {
    /// Producer sequence; each report must exceed the previous one.
    pub seq: ReportSeq,
    /// Declared root state.
    pub state: AgentState,
    /// Turn the report belongs to.
    pub turn: TurnNumber,
    /// Highest input sequence written to the program when the report was parsed.
    pub input: InputSeq,
}

/// The turn a report belongs to and the input the program had received when it was parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TurnPosition {
    /// Turn the report belongs to.
    pub turn: TurnNumber,
    /// Highest input sequence written to the program.
    pub input: InputSeq,
}

impl TurnPosition {
    /// Turn `turn` with input written through `input`.
    pub const fn new(turn: TurnNumber, input: InputSeq) -> Self {
        Self { turn, input }
    }
}

impl StatusFact {
    /// A report of `state` at `seq`, positioned at `at`.
    pub const fn new(seq: ReportSeq, state: AgentState, at: TurnPosition) -> Self {
        Self {
            seq,
            state,
            turn: at.turn,
            input: at.input,
        }
    }

    /// Whether the program declares it is still working on its turn.
    pub(super) const fn active(&self) -> bool {
        matches!(self.state, AgentState::Working | AgentState::Background)
    }
}

/// How a turn ended, from the state its last report left working on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ending {
    Completed,
    Blocked,
    Failed,
}

impl Ending {
    const fn of(state: AgentState) -> Self {
        match state {
            AgentState::Blocked => Self::Blocked,
            AgentState::Error => Self::Failed,
            _ => Self::Completed,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Turn {
    pub number: TurnNumber,
    pub after_input: InputSeq,
    pub ended: Option<(Ending, ReportSeq)>,
}

#[derive(Debug)]
pub(super) struct Facts {
    pub process: ProcessState,
    pub status: Option<StatusFact>,
    pub turns: VecDeque<Turn>,
}

impl Facts {
    pub(super) const fn new() -> Self {
        Self {
            process: ProcessState::Running,
            status: None,
            turns: VecDeque::new(),
        }
    }

    /// Applies an accepted report; a start or end of the working state moves the turns.
    pub(super) fn report(&mut self, fact: StatusFact) -> Result<(), WaitError> {
        if self.process != ProcessState::Running {
            return Err(WaitError::NotRunning {
                process: self.process,
            });
        }
        let previous = self.status;
        if let Some(current) = previous.filter(|current| current.seq >= fact.seq) {
            return Err(WaitError::StaleReport {
                received: fact.seq,
                current: current.seq,
            });
        }
        let was_active = previous.is_some_and(|status| status.active());
        if !was_active && fact.active() {
            self.start(fact)?;
        } else if was_active
            && !fact.active()
            && let Some(turn) = self.turns.back_mut()
        {
            turn.ended = Some((Ending::of(fact.state), fact.seq));
        }
        self.status = Some(fact);
        Ok(())
    }

    fn start(&mut self, fact: StatusFact) -> Result<(), WaitError> {
        if let Some(last) = self.turns.back().filter(|last| last.number >= fact.turn) {
            return Err(WaitError::StaleTurn {
                received: fact.turn,
                current: last.number,
            });
        }
        if self.turns.len() == TURN_HISTORY {
            self.turns.pop_front();
        }
        self.turns.push_back(Turn {
            number: fact.turn,
            after_input: fact.input,
            ended: None,
        });
        Ok(())
    }

    pub(super) fn latest_turn(&self) -> Option<TurnNumber> {
        self.turns.back().map(|turn| turn.number)
    }
}
