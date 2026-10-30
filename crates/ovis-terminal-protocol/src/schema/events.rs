//! Server notifications: the single event envelope, heartbeats and loss notices.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::{
    AgentId, LeaseId, MachineId, PromptId, RuntimeLifetimeId, ServerBootId, StreamId,
    SubscriptionId, TerminalId,
};
use super::methods::events::MissedReason;
use super::records::{FinishedRecord, LeaseRecord, ProcessRecord};
use super::scalar::{
    ByteOffset, DurationMs, EventSeq, Incarnation, OpenEnum, Revision, TimestampMs,
};

/// Resources an event is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventTarget {
    /// Terminal.
    pub terminal_id: Option<TerminalId>,
    /// Terminal incarnation.
    pub terminal_incarnation: Option<Incarnation>,
    /// Program incarnation.
    pub process_incarnation: Option<Incarnation>,
    /// Agent.
    pub agent_id: Option<AgentId>,
    /// Prompt.
    pub prompt_id: Option<PromptId>,
    /// Lease.
    pub lease_id: Option<LeaseId>,
    /// Stream.
    pub stream_id: Option<StreamId>,
    /// Machine.
    pub machine_id: Option<MachineId>,
}

/// One committed event; every type shares this envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EventEnvelope {
    /// Runtime-lifetime sequence.
    pub seq: EventSeq,
    /// Runtime lifetime the sequence belongs to.
    pub lifetime: RuntimeLifetimeId,
    /// Control-server boot that published the event.
    pub boot_id: ServerBootId,
    /// Machine that committed the event.
    pub machine_id: MachineId,
    /// Subscription delivering the event.
    pub subscription_id: SubscriptionId,
    /// Commit time.
    pub ts_ms: TimestampMs,
    /// Event type; see `x-events` for the data schema of each type.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Resources the event is about.
    pub target: EventTarget,
    /// Type-specific data.
    pub data: Value,
}

/// Idle heartbeat of one subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Heartbeat {
    /// Subscription.
    pub subscription_id: SubscriptionId,
    /// Highest sequence delivered.
    pub seq: EventSeq,
    /// Highest sequence examined, including non-matching events.
    pub scan_seq: EventSeq,
    /// Runtime lifetime.
    pub lifetime: RuntimeLifetimeId,
    /// Control-server boot.
    pub boot_id: ServerBootId,
}

/// Loss notice: delivery skipped a range and a fresh snapshot is required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct MissedNotice {
    /// Subscription.
    pub subscription_id: SubscriptionId,
    /// First lost sequence.
    pub from_seq: EventSeq,
    /// Why events were lost.
    pub reason: OpenEnum<MissedReason>,
}

/// Data of `terminal.spawned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SpawnedData {
    /// Spawned program.
    pub process: ProcessRecord,
}

/// Data of `terminal.lost` and `terminal.finished`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EndedData {
    /// Finish facts.
    pub finished: FinishedRecord,
    /// Whether the terminal is kept.
    pub keep: bool,
}

/// Data of `terminal.restarting`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RestartingData {
    /// Attempt number.
    pub attempt: u32,
    /// Delay before the attempt.
    pub next_try_ms: DurationMs,
}

/// Data of `terminal.removed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RemovedData {
    /// Sequence of the tombstone.
    pub tombstone_seq: EventSeq,
}

/// Data of `terminal.foreground`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ForegroundData {
    /// Foreground process, or null when none.
    pub process: Option<ProcessRecord>,
}

/// Data of `output.revision`, coalesced per terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OutputRevisionData {
    /// Screen revision.
    pub revision: Revision,
    /// Raw output offset.
    pub output_offset: ByteOffset,
}

/// Data of `lease.acquired`, `lease.revoked` and `lease.geometry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct LeaseEventData {
    /// Lease after the transition.
    pub lease: LeaseRecord,
    /// Lease that displaced it, for revocations by takeover.
    pub displaced_by: Option<LeaseRecord>,
    /// Recorded reason.
    pub reason: Option<String>,
}

/// Why a stream closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamCloseReason {
    /// The client closed it.
    ClientClosed,
    /// The program exited.
    Exited,
    /// The program was killed.
    Killed,
    /// The terminal was lost.
    Lost,
    /// Another client took over.
    TakenOver,
    /// The lease was revoked.
    LeaseRevoked,
    /// The control server restarted.
    ServerRestart,
    /// The client stopped reading.
    StreamStalled,
    /// The client sent an invalid frame.
    FrameError,
}

/// Data of `stream.closed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct StreamClosedData {
    /// Stream.
    pub stream_id: StreamId,
    /// Reason.
    pub reason: OpenEnum<StreamCloseReason>,
    /// Final filtered stream offset.
    pub final_offset: ByteOffset,
}
