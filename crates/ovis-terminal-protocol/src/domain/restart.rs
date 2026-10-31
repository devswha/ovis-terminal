//! Planned control restart records and the identity transition table (plan todo 48).
//!
//! A planned restart replaces the control server, never the programs its holders own:
//! the runtime lifetime, every terminal and process incarnation and the event sequence
//! survive, while the server boot identity and every stream identity change. These
//! records say which operations and waits the boundary interrupted and how each ended;
//! [`identity`] publishes which identity survives which cause.

use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::ids::{RuntimeLifetimeId, ServerBootId, TerminalId};
use crate::schema::methods::server::{RecoveryEvidence, ServerTransitionResult};
use crate::schema::scalar::{ByteCount, ByteOffset, Incarnation, TimestampMs};

pub mod identity;
#[cfg(test)]
mod tests;

/// Steps of a planned restart, in the only order they may happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RestartPhase {
    /// Control serves requests.
    Running,
    /// The restart intent and the preserved incarnations are durable.
    IntentPersisted,
    /// Readiness is withdrawn; new work is refused with `restarting`.
    Suspended,
    /// The boundary was announced and armed waits were interrupted.
    Announced,
    /// The old control released holder ownership without signalling a program.
    HandedOver,
    /// The successor control adopted the holder under the new boot identity.
    Booted,
    /// Preserved incarnations were verified and interrupted work classified.
    Recovered,
}

impl RestartPhase {
    /// The phase that must follow this one; `Recovered` returns to `Running`.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Running => Self::IntentPersisted,
            Self::IntentPersisted => Self::Suspended,
            Self::Suspended => Self::Announced,
            Self::Announced => Self::HandedOver,
            Self::HandedOver => Self::Booted,
            Self::Booted => Self::Recovered,
            Self::Recovered => Self::Running,
        }
    }

    /// Whether the old control still owns the holder, so a failure can roll back.
    #[must_use]
    pub const fn before_handover(self) -> bool {
        matches!(
            self,
            Self::Running | Self::IntentPersisted | Self::Suspended | Self::Announced
        )
    }
}

/// Which side of a restart boundary an announcement marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundaryPhase {
    /// Readiness was withdrawn; streams of the old boot end with `server_restart`.
    Started,
    /// The successor verified the preserved incarnations; readiness returns.
    Completed,
}

/// One announced restart boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RestartBoundary {
    /// Which side of the boundary.
    pub phase: BoundaryPhase,
    /// The runtime lifetime, unchanged across the boundary.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Boot identity being replaced.
    pub server_boot_id_old: ServerBootId,
    /// Boot identity of the successor.
    pub server_boot_id_new: ServerBootId,
    /// Recovery evidence, once the successor completed.
    pub evidence: Option<RecoveryEvidence>,
}

impl RestartBoundary {
    /// The boundary `phase` from `old` to `new` within `lifetime`, without evidence yet.
    #[must_use]
    pub fn new(
        phase: BoundaryPhase,
        lifetime: RuntimeLifetimeId,
        (old, new): (ServerBootId, ServerBootId),
    ) -> Self {
        Self {
            phase,
            runtime_lifetime: lifetime,
            server_boot_id_old: old,
            server_boot_id_new: new,
            evidence: None,
        }
    }
}

/// What the holder proves about an interrupted input write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Delivery {
    /// No holder acknowledgement was recorded; the bytes may or may not have arrived.
    Unproven {},
    /// The holder acknowledged a write; a prefix shorter than the input is still partial.
    Written {
        /// Bytes the holder wrote to the PTY.
        written_bytes: ByteCount,
        /// Output cursor the holder reported with the acknowledgement.
        output_offset: ByteOffset,
    },
}

/// How an operation that was in flight at the boundary ended, from the ledger alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum InterruptedOutcome {
    /// The ledger committed it before the boundary; its result is retained.
    Committed {},
    /// It was admitted and never started; repeating its `op_id` is safe.
    NotStarted {},
    /// The ledger recorded a failure without effects.
    Failed {},
    /// The ledger recorded a cancellation before commit.
    Cancelled {},
    /// Effects may have begun; it is never repeated automatically.
    OutcomeUnknown {
        /// Holder evidence about the write, when the operation wrote input.
        delivery: Delivery,
    },
}

/// One interrupted operation and its classified outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InterruptedOp {
    /// Operation identity.
    pub op_id: OpId,
    /// How it ended.
    pub outcome: InterruptedOutcome,
}

impl InterruptedOp {
    /// Names `outcome` for `op_id`.
    #[must_use]
    pub const fn new(op_id: OpId, outcome: InterruptedOutcome) -> Self {
        Self { op_id, outcome }
    }

    /// Whether the caller must inspect `op.get` before deciding to repeat it.
    #[must_use]
    pub const fn uncertain(&self) -> bool {
        matches!(self.outcome, InterruptedOutcome::OutcomeUnknown { .. })
    }
}

/// A wait armed when the boundary started: it ended `cancelled`, never `timeout` or a match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InterruptedWait {
    /// The waiting operation.
    pub op_id: OpId,
    /// Terminal the wait observed.
    pub terminal_id: TerminalId,
    /// Output offset a new wait resumes from on the successor.
    pub resume_cursor: ByteOffset,
}

impl InterruptedWait {
    /// A wait of `op_id` on `terminal_id` that resumes at `resume_cursor`.
    #[must_use]
    pub const fn new(op_id: OpId, terminal_id: TerminalId, resume_cursor: ByteOffset) -> Self {
        Self {
            op_id,
            terminal_id,
            resume_cursor,
        }
    }
}

impl ServerTransitionResult {
    /// A restart from `old` to `new` that preserved `preserved` and ended nothing.
    #[must_use]
    pub fn restarted(
        (old, new): (ServerBootId, ServerBootId),
        preserved: Vec<Incarnation>,
        interrupted: Vec<OpId>,
    ) -> Self {
        Self {
            server_boot_id_old: old,
            server_boot_id_new: Some(new),
            preserved_incarnations: preserved,
            ended_incarnations: Vec::new(),
            interrupted_ops: interrupted,
        }
    }

    /// A stop of `old` that left `preserved` running and ended `ended`.
    #[must_use]
    pub const fn stopped(
        old: ServerBootId,
        preserved: Vec<Incarnation>,
        ended: Vec<Incarnation>,
    ) -> Self {
        Self {
            server_boot_id_old: old,
            server_boot_id_new: None,
            preserved_incarnations: preserved,
            ended_incarnations: ended,
            interrupted_ops: Vec::new(),
        }
    }
}

impl RecoveryEvidence {
    /// Evidence that `server_boot_id` restored `restored` with no sequence gap, at `recorded_ms`.
    #[must_use]
    pub const fn restored(
        server_boot_id: ServerBootId,
        restored: Vec<Incarnation>,
        recorded_ms: TimestampMs,
    ) -> Self {
        Self {
            server_boot_id,
            restored,
            gaps: Vec::new(),
            recorded_ms,
        }
    }
}
