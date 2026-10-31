//! One input entry's receipt and the facts each stage may hold.

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, Revision, TimestampMs};
use serde::{Deserialize, Serialize};

use super::InputSeq;
use crate::input::encoding::InputKind;

/// Why no byte of an entry was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FenceReason {
    /// The terminal, process or holder session the entry was admitted for was replaced.
    StaleTarget,
    /// The writer's credential was revoked after admission.
    CredentialRevoked,
    /// The writer's credential expired after admission.
    CredentialExpired,
    /// The writer's grant no longer covers the write.
    Forbidden,
    /// The interact lease the entry was admitted under was released or taken over.
    LeaseRevoked,
    /// A lease appeared on a terminal the entry was admitted to without one.
    NoLease,
    /// The raw-input grant lapsed after admission.
    RawNotAuthorized,
    /// The terminal modes moved past the revision the bytes were encoded against.
    ModeMismatch,
    /// The holder link was lost before the entry was sent.
    Recovering,
    /// The program exited before the entry was sent.
    Finished,
    /// The holder's input queue had no room.
    InputOverflow,
    /// The holder acknowledged the entry without writing it.
    Unwritten,
}

impl FenceReason {
    /// The stable machine-readable token clients branch on.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::StaleTarget => "stale_target",
            Self::CredentialRevoked => "credential_revoked",
            Self::CredentialExpired => "credential_expired",
            Self::Forbidden => "forbidden",
            Self::LeaseRevoked => "lease_revoked",
            Self::NoLease => "no_lease",
            Self::RawNotAuthorized => "raw_not_authorized",
            Self::ModeMismatch => "mode_mismatch",
            Self::Recovering => "recovering",
            Self::Finished => "finished",
            Self::InputOverflow => "input_overflow",
            Self::Unwritten => "unwritten",
        }
    }
}

/// What happened to an entry's bytes; only holder evidence moves it past `queued`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Delivery {
    /// Admitted; no write evidence yet.
    Queued,
    /// Every byte reached the PTY.
    PtyWritten {
        /// Bytes written.
        written_bytes: ByteCount,
        /// Output offset the holder saw when the write started.
        output_offset: ByteOffset,
    },
    /// Only a prefix reached the PTY; the rest will never be written.
    Partial {
        /// Exact prefix length written.
        written_bytes: ByteCount,
        /// Output offset the holder saw when the write started.
        output_offset: ByteOffset,
    },
    /// No byte was written.
    Fenced {
        /// Why.
        reason: FenceReason,
    },
    /// The entry was sent but evidence ended before the holder acknowledged it; never resent.
    OutcomeUnknown,
}

/// Whether a producer is declared that can report an agent stage for the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reporter {
    /// No producer can report; agent stages stay unknown.
    Undeclared,
    /// A declared producer may report; agent stages are pending until it does.
    Declared,
}

/// An agent-side stage after delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentStage {
    /// The agent acknowledged the submission.
    Accepted,
    /// The agent started a turn for it.
    TurnStarted,
    /// That turn ended.
    TurnEnded,
}

/// Evidence for one agent stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Stage {
    /// No producer can report this stage for the entry.
    Unknown,
    /// A declared producer has not reported yet.
    Pending,
    /// The producer reported the stage.
    Reported {
        /// When the producer reported it.
        at_ms: TimestampMs,
    },
}

/// A producer's report of one stage of one admitted entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StageReport {
    /// Operation the report refers to.
    pub op_id: OpId,
    /// Stage reported.
    pub stage: AgentStage,
    /// When it was reported.
    pub at: TimestampMs,
}

impl StageReport {
    /// Reports `stage` of `op_id` at `at`.
    pub const fn new(op_id: OpId, stage: AgentStage, at: TimestampMs) -> Self {
        Self { op_id, stage, at }
    }
}

/// The staged receipt of one admitted input entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Receipt {
    /// Operation identity.
    pub op_id: OpId,
    /// Position in the terminal's input sequence.
    pub queued_seq: InputSeq,
    /// Input path.
    pub kind: InputKind,
    /// Bytes admitted, text and submission together.
    pub admitted_bytes: ByteCount,
    /// Mode revision the bytes were encoded against.
    pub mode_revision: Revision,
    /// Output offset captured with admission.
    pub output_cursor: ByteOffset,
    /// Write evidence.
    pub delivery: Delivery,
    /// Agent acknowledgement.
    pub agent_accepted: Stage,
    /// Turn start.
    pub turn_started: Stage,
    /// Turn end.
    pub turn_ended: Stage,
}

impl Receipt {
    pub(in crate::input) const fn stage_mut(&mut self, stage: AgentStage) -> &mut Stage {
        match stage {
            AgentStage::Accepted => &mut self.agent_accepted,
            AgentStage::TurnStarted => &mut self.turn_started,
            AgentStage::TurnEnded => &mut self.turn_ended,
        }
    }
}
