//! Waits for a declared state or the process exit.

use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::ReportSeq;
use serde::{Deserialize, Serialize};

use super::board::Board;
use super::entry::{Armed, Condition, Entry, Start, Watch};
use super::error::WaitError;
use super::facts::{ExitEvidence, Facts, ProcessState};

/// Wait until the program declares one of `states`, or, when `exit` is set, exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateWait {
    states: Vec<AgentState>,
    exit: bool,
}

impl StateWait {
    /// A wait for any of `states`, or for the exit when `exit` is set.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::EmptyCondition`] when it names no state and no exit.
    pub fn new(states: &[AgentState], exit: bool) -> Result<Self, WaitError> {
        if states.is_empty() && !exit {
            return Err(WaitError::EmptyCondition);
        }
        Ok(Self {
            states: states.to_vec(),
            exit,
        })
    }

    fn satisfied(&self, facts: &Facts) -> Option<Satisfied> {
        match facts.process {
            ProcessState::Running => facts
                .status
                .filter(|status| self.states.contains(&status.state))
                .map(|status| Satisfied::Status {
                    seq: status.seq,
                    state: status.state,
                }),
            ProcessState::Exited(exit) => self.exit.then_some(Satisfied::Exit { exit }),
            ProcessState::Lost => None,
        }
    }

    const fn ended(facts: &Facts) -> Option<StateOutcome> {
        match facts.process {
            ProcessState::Running => None,
            ProcessState::Exited(exit) => Some(StateOutcome::Exited { exit }),
            ProcessState::Lost => Some(StateOutcome::Lost),
        }
    }
}

/// The fact that satisfied a state wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Satisfied {
    /// A declared status report in the requested set.
    Status {
        /// Report sequence.
        seq: ReportSeq,
        /// Declared state.
        state: AgentState,
    },
    /// The process exit the wait asked for.
    Exit {
        /// The holder's exit report.
        exit: ExitEvidence,
    },
}

/// How a state wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum StateOutcome {
    /// The condition already held when the wait was registered; nothing was armed.
    AlreadySatisfied {
        /// The deciding fact.
        satisfied: Satisfied,
    },
    /// A fact fed after registration satisfied the condition.
    Matched {
        /// The deciding fact.
        satisfied: Satisfied,
    },
    /// The process exited, which the wait did not ask for.
    Exited {
        /// The holder's exit report.
        exit: ExitEvidence,
    },
    /// The holder link ended.
    Lost,
    /// The deadline passed; the condition never held.
    Timeout,
    /// The wait's scope was cancelled.
    Cancelled,
}

impl Watch for StateWait {
    type Outcome = StateOutcome;

    fn settle(&mut self, facts: &Facts) -> Option<StateOutcome> {
        self.satisfied(facts)
            .map(|satisfied| StateOutcome::Matched { satisfied })
            .or_else(|| Self::ended(facts))
    }

    fn expire(&self, _facts: &Facts) -> StateOutcome {
        StateOutcome::Timeout
    }

    fn cancelled(&self, _facts: &Facts) -> StateOutcome {
        StateOutcome::Cancelled
    }
}

impl Condition for StateWait {
    type Watch = Self;

    fn start(self, board: &Board) -> Result<Start<Self>, WaitError> {
        if let Some(satisfied) = self.satisfied(&board.facts) {
            return Ok(Start::Done(StateOutcome::AlreadySatisfied { satisfied }));
        }
        Ok(Start::settled(self, &board.facts))
    }

    fn cancelled(&self, _facts: &Facts) -> StateOutcome {
        StateOutcome::Cancelled
    }

    fn entry(armed: Armed<Self>) -> Entry {
        Entry::State(armed)
    }
}
