//! Turn waits bound to one send or one turn number.

use ovis_terminal_protocol::schema::scalar::{ReportSeq, TurnNumber};
use serde::{Deserialize, Serialize};

use super::board::Board;
use super::entry::{Armed, Condition, Entry, Start, Watch};
use super::error::WaitError;
use super::facts::{Ending, ExitEvidence, Facts, ProcessState, Turn};
use crate::input::receipts::{InputSeq, Receipt};

/// The turn a wait is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnBinding {
    /// The first turn that started after the program received this input entry.
    Send(InputSeq),
    /// This turn number.
    Turn(TurnNumber),
}

impl TurnBinding {
    /// The turn answering the entry `receipt` admitted.
    pub const fn send(receipt: &Receipt) -> Self {
        Self::Send(receipt.queued_seq)
    }

    fn find(self, facts: &Facts) -> Option<&Turn> {
        match self {
            Self::Send(seq) => facts.turns.iter().find(|turn| turn.after_input >= seq),
            Self::Turn(number) => facts.turns.iter().find(|turn| turn.number == number),
        }
    }

    fn check(self, facts: &Facts) -> Result<(), WaitError> {
        let Self::Turn(turn) = self else {
            return Ok(());
        };
        let retained = self.find(facts).is_some();
        let passed = facts.latest_turn().is_some_and(|latest| latest >= turn);
        if !retained && passed {
            return Err(WaitError::UnknownTurn { turn });
        }
        Ok(())
    }
}

/// How a bound turn wait ended; `turn` is absent when no bound turn started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum TurnOutcome {
    /// The bound turn left working on a non-blocked, non-error state.
    Completed {
        /// The turn.
        turn: TurnNumber,
        /// Report that ended it.
        ended_seq: ReportSeq,
    },
    /// The bound turn ended blocked on a prompt.
    Blocked {
        /// The turn.
        turn: TurnNumber,
        /// Report that ended it.
        ended_seq: ReportSeq,
    },
    /// The bound turn ended on an error report.
    Failed {
        /// The turn.
        turn: TurnNumber,
        /// Report that ended it.
        ended_seq: ReportSeq,
    },
    /// The process exited before the bound turn ended.
    Exited {
        /// The bound turn, when it started.
        turn: Option<TurnNumber>,
        /// The holder's exit report.
        exit: ExitEvidence,
    },
    /// The holder link ended before the bound turn ended.
    Lost {
        /// The bound turn, when it started.
        turn: Option<TurnNumber>,
    },
    /// The wait's scope was cancelled.
    Cancelled {
        /// The bound turn, when it started.
        turn: Option<TurnNumber>,
    },
    /// The deadline passed; the bound turn did not end, or never started.
    Timeout {
        /// The bound turn, when it started.
        turn: Option<TurnNumber>,
    },
}

impl TurnOutcome {
    const fn ended(turn: TurnNumber, ending: Ending, ended_seq: ReportSeq) -> Self {
        match ending {
            Ending::Completed => Self::Completed { turn, ended_seq },
            Ending::Blocked => Self::Blocked { turn, ended_seq },
            Ending::Failed => Self::Failed { turn, ended_seq },
        }
    }
}

impl Watch for TurnBinding {
    type Outcome = TurnOutcome;

    fn settle(&mut self, facts: &Facts) -> Option<TurnOutcome> {
        let bound = self.find(facts);
        if let Some((turn, (ending, seq))) =
            bound.and_then(|bound| bound.ended.map(|ended| (bound.number, ended)))
        {
            return Some(TurnOutcome::ended(turn, ending, seq));
        }
        let turn = bound.map(|bound| bound.number);
        match facts.process {
            ProcessState::Running => None,
            ProcessState::Exited(exit) => Some(TurnOutcome::Exited { turn, exit }),
            ProcessState::Lost => Some(TurnOutcome::Lost { turn }),
        }
    }

    fn expire(&self, facts: &Facts) -> TurnOutcome {
        TurnOutcome::Timeout {
            turn: self.find(facts).map(|bound| bound.number),
        }
    }

    fn cancelled(&self, facts: &Facts) -> TurnOutcome {
        TurnOutcome::Cancelled {
            turn: self.find(facts).map(|bound| bound.number),
        }
    }
}

impl Condition for TurnBinding {
    type Watch = Self;

    fn start(self, board: &Board) -> Result<Start<Self>, WaitError> {
        self.check(&board.facts)?;
        Ok(Start::settled(self, &board.facts))
    }

    fn cancelled(&self, facts: &Facts) -> TurnOutcome {
        Watch::cancelled(self, facts)
    }

    fn entry(armed: Armed<Self>) -> Entry {
        Entry::Turn(armed)
    }
}
