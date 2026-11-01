//! Per-machine actors on the wire (plan todo 44): line health, remote events mirrored into
//! the local sequence domain, and orphan notices delivered until acknowledged.
//!
//! Remote ordering evidence (`seq`, `ts_ms`) is carried beside the local sequence and is
//! never used to order local events; clock skew is reported, not corrected. Orphan notices
//! are named by opaque tokens the sender mints, so a replay after reconnection carries the
//! same notification id and a receiver applies it at most once.

mod orphan;
mod skew;

pub use orphan::{
    AckOutcome, Delivery, NotificationId, ORPHANED, ORPHANED_ACK, OrphanedAckParams,
    OrphanedAckResult, OrphanedParams, OrphanedResult, ParentIncarnation, TokenError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use skew::{SkewError, SkewMs};

use crate::errors::ErrorKind;
use crate::schema::ids::{MachineId, RuntimeLifetimeId, TerminalId};
use crate::schema::methods::machine::{MachineListResult, MachineRecord, MachineState};
use crate::schema::scalar::{Count, EventSeq, OpenEnum, Revision, TimestampMs};

/// Notification a remote runtime sends for each committed event.
pub const EVENT: &str = "event";
/// Notification a remote runtime sends when it examined events without one to send.
pub const HEARTBEAT: &str = "heartbeat";

/// Why a machine's line last failed, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LineFailure {
    /// Stable failure token, such as `deadline_exceeded` or `transport_failed`.
    pub kind: ErrorKind,
    /// Local wall clock of the failure.
    pub at_ms: TimestampMs,
}

impl LineFailure {
    /// A failure of `kind` observed at `at_ms`.
    #[must_use]
    pub const fn new(kind: ErrorKind, at_ms: TimestampMs) -> Self {
        Self { kind, at_ms }
    }
}

/// Queryable state of one machine's actor; it outlives any notification about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MachineHealth {
    /// The machine.
    pub machine_id: MachineId,
    /// Line state.
    pub state: OpenEnum<MachineState>,
    /// Lines attached so far; each reconnection starts a new one.
    pub line_epoch: Count,
    /// Advances on every change of this record.
    pub revision: Revision,
    /// The last failure, kept after recovery.
    pub last_failure: Option<LineFailure>,
    /// Calls that ran out of their deadline.
    pub deadline_exceeded: Count,
    /// Calls sent and not answered yet.
    pub in_flight: Count,
    /// Last remote sequence mirrored locally.
    pub last_seen_seq: Option<EventSeq>,
    /// Remote notifications the local consumer had no room for; the mirror reports them
    /// as a gap.
    pub inbound_dropped: Count,
}

impl MachineHealth {
    /// A machine whose first line has not been attached.
    #[must_use]
    pub const fn detached(machine_id: MachineId) -> Self {
        Self {
            machine_id,
            state: OpenEnum::Known(MachineState::Connecting),
            line_epoch: Count::new(0),
            revision: Revision::new(0),
            last_failure: None,
            deadline_exceeded: Count::new(0),
            in_flight: Count::new(0),
            last_seen_seq: None,
            inbound_dropped: Count::new(0),
        }
    }
}

impl MachineRecord {
    /// This identity record with the line state of `health`; the record revision advances
    /// with every identity or line change.
    #[must_use]
    pub fn observed(&self, health: &MachineHealth) -> Self {
        Self {
            state: health.state.clone(),
            last_seen_seq: health.last_seen_seq.or(self.last_seen_seq),
            revision: Revision::new(self.revision.get().saturating_add(health.revision.get())),
            ..self.clone()
        }
    }
}

/// A remote terminal an event describes, with the revision the event left it at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RemoteTerminal {
    /// Terminal identity on the remote machine.
    pub terminal_id: TerminalId,
    /// Revision after the event.
    pub revision: Revision,
    /// Whether the event removed the terminal.
    pub removed: bool,
}

impl RemoteTerminal {
    /// `terminal_id` at `revision`, removed or not.
    #[must_use]
    pub const fn new(terminal_id: TerminalId, revision: Revision, removed: bool) -> Self {
        Self {
            terminal_id,
            revision,
            removed,
        }
    }
}

/// Parameters of the `event` notification a remote runtime sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RemoteEvent {
    /// Remote sequence.
    pub seq: EventSeq,
    /// Remote runtime lifetime owning `seq`.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Remote wall clock; evidence only.
    pub ts_ms: TimestampMs,
    /// Remote event type.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The remote terminal the event changed, if any.
    pub terminal: Option<RemoteTerminal>,
    /// Remote event data, kept verbatim.
    pub data: Value,
}

/// Parameters of the `heartbeat` notification: every remote sequence through `scan_seq`
/// was examined and nothing more was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RemoteScan {
    /// Remote runtime lifetime owning `scan_seq`.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Highest remote sequence examined.
    pub scan_seq: EventSeq,
}

/// Where a mirrored record came from: machine, remote lifetime and remote sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MirrorSource {
    /// Originating machine.
    pub machine_id: MachineId,
    /// Originating runtime lifetime.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Remote sequence, never a local one.
    pub seq: EventSeq,
}

impl MirrorSource {
    /// Position `seq` of `runtime_lifetime` on `machine_id`.
    #[must_use]
    pub const fn new(
        machine_id: MachineId,
        runtime_lifetime: RuntimeLifetimeId,
        seq: EventSeq,
    ) -> Self {
        Self {
            machine_id,
            runtime_lifetime,
            seq,
        }
    }
}

/// A remote event mirrored into the local sequence domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MirroredEvent {
    /// Remote identity and cursor.
    pub source: MirrorSource,
    /// Remote wall clock, as sent.
    pub remote_ts_ms: TimestampMs,
    /// Local wall clock at receipt.
    pub received_ms: TimestampMs,
    /// `remote_ts_ms - received_ms`: metadata, never an ordering key.
    pub clock_skew_ms: SkewMs,
    /// Remote event type.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The remote terminal the event changed, if any.
    pub terminal: Option<RemoteTerminal>,
    /// Remote event data, verbatim.
    pub data: Value,
}

impl MirroredEvent {
    /// `event` from `machine_id` received at `received_ms`; its skew is reported, never used.
    #[must_use]
    pub fn received(machine_id: MachineId, event: RemoteEvent, received_ms: TimestampMs) -> Self {
        Self {
            source: MirrorSource::new(machine_id, event.runtime_lifetime, event.seq),
            remote_ts_ms: event.ts_ms,
            received_ms,
            clock_skew_ms: SkewMs::between(event.ts_ms, received_ms),
            event_type: event.event_type,
            terminal: event.terminal,
            data: event.data,
        }
    }
}

impl MachineListResult {
    /// The roster of `machines`.
    #[must_use]
    pub const fn new(machines: Vec<MachineRecord>) -> Self {
        Self { machines }
    }
}

/// What a mirror boundary marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundaryKind {
    /// Remote sequences through `through` were examined without a matching event.
    Scan,
    /// Remote sequences up to `through` were never received.
    Gap,
    /// The remote runtime lifetime changed; earlier cursors do not continue.
    LifetimeChanged,
    /// The line was lost after `last_seen`; resources keep their last-known revision.
    LineLost,
    /// A new line was attached; mirroring continues after `last_seen`.
    Resumed,
}

/// An explicit scan or loss boundary in a machine's mirrored stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MirrorBoundary {
    /// The machine.
    pub machine_id: MachineId,
    /// Boundary kind.
    pub kind: OpenEnum<BoundaryKind>,
    /// Last remote position mirrored before the boundary.
    pub last_seen: Option<MirrorSource>,
    /// Last remote sequence the boundary covers, for scans and gaps.
    pub through: Option<EventSeq>,
}

impl MirrorBoundary {
    /// A `kind` boundary on `machine_id` after `last_seen`, covering through `through`.
    #[must_use]
    pub fn new(
        (machine_id, kind): (MachineId, BoundaryKind),
        last_seen: Option<MirrorSource>,
        through: Option<EventSeq>,
    ) -> Self {
        Self {
            machine_id,
            kind: OpenEnum::Known(kind),
            last_seen,
            through,
        }
    }
}

#[cfg(test)]
#[path = "machine_actor/tests.rs"]
mod tests;
