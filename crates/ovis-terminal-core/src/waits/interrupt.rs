//! Interrupt outcomes: the delivered action and the observed stop are separate facts.

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::input::TurnObservation;
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::ReportSeq;
use serde::Serialize;

use super::board::Board;
use super::entry::{Armed, Condition, Entry, Start, Watch};
use super::error::WaitError;
use super::facts::{ExitEvidence, Facts, ProcessState};
use crate::input::receipts::{Delivery, InputSeq, Receipt};

/// Arms stop observation before the interrupt key is admitted, so no evidence is missed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StopWatch;

impl StopWatch {
    /// Observation starting at the facts current when it is armed.
    pub const fn new() -> Self {
        Self
    }
}

/// Evidence that the program stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum StopEvidence {
    /// The holder reported the exit of this process incarnation.
    Exit {
        /// The holder's exit report.
        exit: ExitEvidence,
    },
    /// A declared report left the working state after observation was armed.
    Status {
        /// Report sequence.
        seq: ReportSeq,
        /// The state it left working for.
        state: AgentState,
    },
}

/// Why observation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum WatchEnd {
    /// Stop evidence arrived.
    Stopped {
        /// The deciding evidence.
        evidence: StopEvidence,
    },
    /// The holder link ended; nothing more can be observed.
    Lost,
    /// The observation bound passed without stop evidence.
    Timeout,
    /// The request was cancelled.
    Cancelled,
}

/// Process liveness when observation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProcessObservation {
    /// No exit was reported.
    Running,
    /// The holder reported the exit.
    Exited,
    /// The holder link ended; liveness is unknown.
    Unknown,
}

/// What was observed after the interrupt, independent of its delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Observed {
    /// Why observation ended.
    pub end: WatchEnd,
    /// Process liveness.
    pub process: ProcessObservation,
    /// Turn state from declared status only.
    pub turn: TurnObservation,
    /// Newest declared report sequence.
    pub status_seq: Option<ReportSeq>,
}

impl Observed {
    fn of(end: WatchEnd, facts: &Facts) -> Self {
        let process = match facts.process {
            ProcessState::Running => ProcessObservation::Running,
            ProcessState::Exited(_) => ProcessObservation::Exited,
            ProcessState::Lost => ProcessObservation::Unknown,
        };
        let turn = match (end, facts.status) {
            (WatchEnd::Stopped { .. }, _) => TurnObservation::Ended,
            (_, Some(status)) if status.active() => TurnObservation::Working,
            _ => TurnObservation::Unknown,
        };
        Self {
            end,
            process,
            turn,
            status_seq: facts.status.map(|status| status.seq),
        }
    }

    /// Whether the evidence proves the program stopped.
    pub const fn stopped(&self) -> bool {
        matches!(self.end, WatchEnd::Stopped { .. })
    }
}

#[derive(Debug)]
pub(super) struct Baseline {
    seq: Option<ReportSeq>,
    working: bool,
}

impl Watch for Baseline {
    type Outcome = Observed;

    fn settle(&mut self, facts: &Facts) -> Option<Observed> {
        let end = match facts.process {
            ProcessState::Exited(exit) => WatchEnd::Stopped {
                evidence: StopEvidence::Exit { exit },
            },
            ProcessState::Lost => WatchEnd::Lost,
            ProcessState::Running => {
                let status = facts
                    .status
                    .filter(|status| self.seq.is_none_or(|seq| status.seq > seq))?;
                if status.active() {
                    self.working = true;
                    return None;
                }
                if !self.working {
                    return None;
                }
                WatchEnd::Stopped {
                    evidence: StopEvidence::Status {
                        seq: status.seq,
                        state: status.state,
                    },
                }
            }
        };
        Some(Observed::of(end, facts))
    }

    fn expire(&self, facts: &Facts) -> Observed {
        Observed::of(WatchEnd::Timeout, facts)
    }

    fn cancelled(&self, facts: &Facts) -> Observed {
        Observed::of(WatchEnd::Cancelled, facts)
    }
}

impl Condition for StopWatch {
    type Watch = Baseline;

    fn start(self, board: &Board) -> Result<Start<Baseline>, WaitError> {
        let facts = &board.facts;
        let baseline = Baseline {
            seq: facts.status.map(|status| status.seq),
            working: facts.status.is_some_and(|status| status.active()),
        };
        Ok(Start::settled(baseline, facts))
    }

    fn cancelled(&self, facts: &Facts) -> Observed {
        Observed::of(WatchEnd::Cancelled, facts)
    }

    fn entry(armed: Armed<Baseline>) -> Entry {
        Entry::Stop(armed)
    }
}

const fn is_delivered(delivery: Delivery) -> bool {
    match delivery {
        Delivery::PtyWritten { written_bytes, .. } | Delivery::Partial { written_bytes, .. } => {
            written_bytes.get() > 0
        }
        Delivery::Queued | Delivery::Fenced { .. } | Delivery::OutcomeUnknown => false,
    }
}

/// An interrupt's result: what was delivered and what was observed, never merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct InterruptReport {
    /// Operation that admitted the key.
    pub op_id: OpId,
    /// Position of the key in the input sequence.
    pub queued_seq: InputSeq,
    /// Whether key bytes reached the program, from holder write evidence.
    pub delivered: bool,
    /// Write evidence for the key.
    pub delivery: Delivery,
    /// What was observed.
    pub observed: Observed,
    /// True only with exit or declared-status stop evidence.
    pub stopped: bool,
}

impl InterruptReport {
    /// Joins the key's receipt with the observation armed before it was admitted.
    pub fn new(receipt: &Receipt, observed: Observed) -> Self {
        Self {
            op_id: receipt.op_id.clone(),
            queued_seq: receipt.queued_seq,
            delivered: is_delivered(receipt.delivery),
            delivery: receipt.delivery,
            observed,
            stopped: observed.stopped(),
        }
    }
}
