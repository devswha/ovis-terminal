//! Turn facts read from the agent status turn, never estimated.
//!
//! Pure: the status row is the only input. A start time is known only while the status
//! root is the report that started the turn, and an end time only while it is the
//! report that ended it; otherwise the time stays null instead of borrowing a later
//! report's time, which would stretch the turn over idle periods.

use ovis_terminal_protocol::domain::conversation_records::{ConversationTurn, TurnState};
use ovis_terminal_protocol::schema::methods::status::TurnOutcome;
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum, TimestampMs, TurnNumber};
use serde::{Deserialize, Serialize};

use crate::status::row::Row;

/// What is recorded about one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TurnFacts {
    pub outcome: Option<OpenEnum<TurnOutcome>>,
    pub started_seq: Option<EventSeq>,
    pub ended_seq: Option<EventSeq>,
    pub started_ms: Option<TimestampMs>,
    pub ended_ms: Option<TimestampMs>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
}

/// The turn the status root is working on, if it is working.
pub(super) fn running(row: &Row) -> Option<TurnNumber> {
    let working = row
        .status
        .root
        .as_ref()
        .is_some_and(|root| root.report.state == OpenEnum::Known(AgentState::Working));
    working.then_some(row.status.turn)
}

/// Acceptance time of the status root when it is the report at `seq`.
fn root_time(row: &Row, seq: Option<EventSeq>) -> Option<TimestampMs> {
    let root = row.status.root.as_ref()?;
    (Some(root.seq) == seq).then_some(root.since_ms)
}

impl TurnFacts {
    /// Folds in what the status row records about `turn` now; known facts are kept.
    pub(super) fn observe(&mut self, turn: TurnNumber, row: &Row) {
        if running(row) == Some(turn) {
            self.started_seq = self.started_seq.or(row.started_seq);
            self.started_ms = self.started_ms.or_else(|| root_time(row, row.started_seq));
        }
        if let Some(last) = row
            .status
            .last_turn
            .as_ref()
            .filter(|last| last.turn == turn)
        {
            self.outcome.clone_from(&last.outcome);
            self.started_seq = self.started_seq.or(last.started_seq);
            self.ended_seq = last.ended_seq;
            self.ended_ms = self.ended_ms.or_else(|| root_time(row, last.ended_seq));
        }
    }

    /// `turn` as a read presents it against the status row `row`.
    pub(super) fn view(&self, turn: TurnNumber, row: &Row) -> ConversationTurn {
        let mut facts = self.clone();
        facts.observe(turn, row);
        let state = if running(row) == Some(turn) {
            TurnState::Open
        } else {
            TurnState::Ended
        };
        let mut view = ConversationTurn::new(turn, state);
        view.outcome = facts.outcome;
        view.started_seq = facts.started_seq;
        view.ended_seq = facts.ended_seq;
        view.started_ms = facts.started_ms;
        view.ended_ms = facts.ended_ms;
        view.model = facts.model;
        view.reasoning = facts.reasoning;
        view
    }
}
