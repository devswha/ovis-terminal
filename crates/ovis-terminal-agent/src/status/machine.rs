//! Root and child record transitions, turn numbering and finish generations.
//!
//! Pure: every input, including the accepting event sequence and the time, is supplied
//! by the caller, so a transition never reads a clock.

use ovis_terminal_protocol::schema::methods::status::{TurnOutcome, TurnRecord};
use ovis_terminal_protocol::schema::records::{AgentState, StatusEntry};
use ovis_terminal_protocol::schema::scalar::{EventSeq, FinishGeneration, OpenEnum, TurnNumber};

use super::row::Row;

/// The six states a producer may report; never-reported is `root: null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lifecycle {
    Working,
    Idle,
    Blocked,
    Done,
    Background,
    Error,
}

impl Lifecycle {
    /// Parses a reported state token; unknown tokens are not a lifecycle state.
    pub(super) fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "working" => Self::Working,
            "idle" => Self::Idle,
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            "background" => Self::Background,
            "error" => Self::Error,
            _ => return None,
        })
    }

    /// Parses the wire form of a reported state.
    pub(super) const fn of(state: &OpenEnum<AgentState>) -> Option<Self> {
        match state {
            OpenEnum::Known(AgentState::Working) => Some(Self::Working),
            OpenEnum::Known(AgentState::Idle) => Some(Self::Idle),
            OpenEnum::Known(AgentState::Blocked) => Some(Self::Blocked),
            OpenEnum::Known(AgentState::Done) => Some(Self::Done),
            OpenEnum::Known(AgentState::Background) => Some(Self::Background),
            OpenEnum::Known(AgentState::Error) => Some(Self::Error),
            OpenEnum::Known(_) | OpenEnum::Unknown(_) => None,
        }
    }

    pub(super) const fn state(self) -> AgentState {
        match self {
            Self::Working => AgentState::Working,
            Self::Idle => AgentState::Idle,
            Self::Blocked => AgentState::Blocked,
            Self::Done => AgentState::Done,
            Self::Background => AgentState::Background,
            Self::Error => AgentState::Error,
        }
    }

    const fn outcome(self) -> TurnOutcome {
        match self {
            Self::Working | Self::Idle | Self::Done | Self::Background => TurnOutcome::Completed,
            Self::Blocked => TurnOutcome::Blocked,
            Self::Error => TurnOutcome::Failed,
        }
    }
}

/// A validated report ready to be applied at a sequence.
#[derive(Debug, Clone)]
pub(super) struct Accepted {
    pub entry: StatusEntry,
    pub lifecycle: Lifecycle,
}

/// How the root left `working`, if it did.
#[derive(Debug, Clone, Copy)]
enum Ending {
    Reported(Lifecycle),
    Exited,
}

fn root_working(row: &Row) -> bool {
    row.status
        .root
        .as_ref()
        .is_some_and(|root| Lifecycle::of(&root.report.state) == Some(Lifecycle::Working))
}

impl Row {
    /// Applies an accepted report as the event at `seq`; returns the turn it finished.
    pub(super) fn accept(&mut self, accepted: Accepted, seq: EventSeq) -> Option<TurnRecord> {
        let Accepted {
            mut entry,
            lifecycle,
        } = accepted;
        entry.seq = seq;
        self.status.reported = true;
        self.status.report_seq = Some(entry.report_seq);
        self.last_event = seq;
        if let Some(id) = entry.report.id.clone() {
            entry.turn = self.status.turn;
            self.status.records.insert(id, entry);
            return None;
        }
        let was_working = root_working(self);
        let working = lifecycle == Lifecycle::Working;
        if working && !was_working {
            self.status.turn = TurnNumber::new(self.status.turn.get().saturating_add(1));
            self.started_seq = Some(seq);
        }
        entry.turn = self.status.turn;
        self.status.root = Some(entry);
        (was_working && !working).then(|| self.finish(Ending::Reported(lifecycle), seq))
    }

    /// Removes the child record `id`, or the root without one; a clear ends no turn.
    pub(super) fn clear(&mut self, id: Option<&str>, seq: EventSeq) {
        match id {
            Some(id) => {
                self.status.records.remove(id);
            }
            None => self.status.root = None,
        }
        self.last_event = seq;
    }

    /// Invalidates every record of an exited program; a working root ends its turn.
    pub(super) fn exited(&mut self, seq: EventSeq) -> Option<TurnRecord> {
        let ended = root_working(self).then(|| self.finish(Ending::Exited, seq));
        self.status.root = None;
        self.status.records.clear();
        self.last_event = seq;
        ended
    }

    fn finish(&mut self, ending: Ending, seq: EventSeq) -> TurnRecord {
        let generation = self.status.finish_generation.get().saturating_add(1);
        self.status.finish_generation = FinishGeneration::new(generation);
        let mut turn = TurnRecord::new(self.status.turn);
        let (outcome, ended_on) = match ending {
            Ending::Reported(lifecycle) => (lifecycle.outcome(), Some(lifecycle.state())),
            Ending::Exited => (TurnOutcome::Exited, None),
        };
        turn.outcome = Some(OpenEnum::Known(outcome));
        turn.ended_on = ended_on.map(OpenEnum::Known);
        turn.started_seq = self.started_seq.take();
        turn.ended_seq = Some(seq);
        turn.finish_generation = Some(self.status.finish_generation);
        turn.process_incarnation = self.status.process_incarnation;
        turn.agent_id.clone_from(&self.status.agent_id);
        turn.session.clone_from(&self.status.session);
        self.status.last_turn = Some(turn.clone());
        turn
    }
}
