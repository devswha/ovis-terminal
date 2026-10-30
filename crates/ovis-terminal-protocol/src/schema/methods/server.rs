//! `hello`, `server.*`, `session.*` and `op.*`: connection setup, health and operations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{ErrorData, OpId};
use crate::schema::common::{NoParams, PageInfo, PageRequest};
use crate::schema::ids::{
    BuildVersion, ClientId, MachineId, MethodName, RuntimeLifetimeId, ServerBootId,
};
use crate::schema::limits::{Capabilities, Limits, TransportKind};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{AgentBinding, LeaseRecord, TerminalRecord};
use crate::schema::scalar::{
    ByteCount, DurationMs, EventSeq, Incarnation, OpenEnum, ProtocolGeneration, Revision,
    TimestampMs,
};

/// Client identity presented in `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ClientInfo {
    /// Client program name.
    pub name: String,
    /// Client program version.
    pub version: String,
}

/// Parameters of `hello`, the first request on every connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HelloParams {
    /// Protocol generation the client speaks.
    pub generation: ProtocolGeneration,
    /// Client identity.
    pub client: ClientInfo,
    /// Upgrade the connection to framed mode after this reply.
    pub framed: Option<bool>,
    /// Credential for remote or WebSocket transports.
    pub credential: Option<String>,
}

/// Result of `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct HelloResult {
    /// Stable machine identity.
    pub machine_id: MachineId,
    /// Holder-supervisor lifetime.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Control-server boot identity.
    pub server_boot_id: ServerBootId,
    /// Negotiated protocol generation.
    pub generation: ProtocolGeneration,
    /// Runtime build version.
    pub version: BuildVersion,
    /// Methods this build implements; never a method it would refuse.
    pub methods: Vec<MethodName>,
    /// Served capabilities.
    pub capabilities: Capabilities,
    /// Advertised limits.
    pub limits: Limits,
    /// Server clock.
    pub server_time_ms: TimestampMs,
    /// Transport of this connection.
    pub endpoint_kind: OpenEnum<TransportKind>,
}

/// Observed state of a control server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServerState {
    /// The control server answers.
    Running,
    /// No control server runs.
    Stopped,
    /// The endpoint exists but does not answer.
    Unreachable,
    /// The endpoint names a runtime that is gone.
    StaleEndpoint,
}

/// Result of `server.status`; observing never starts a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ServerStatusResult {
    /// Observed state.
    pub state: OpenEnum<ServerState>,
    /// Holder-supervisor lifetime, when known.
    pub runtime_lifetime: Option<RuntimeLifetimeId>,
    /// Control-server boot identity, when running.
    pub server_boot_id: Option<ServerBootId>,
    /// Uptime, when running.
    pub uptime_ms: Option<DurationMs>,
    /// Live holder processes.
    pub holder_count: u32,
    /// Live terminals.
    pub live_terminals: u32,
}

/// Parameters of `server.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServerStartParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
}

/// Result of `server.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ServerStartResult {
    /// Whether this call started the server.
    pub started: bool,
    /// Resulting boot identity.
    pub server_boot_id: ServerBootId,
    /// Holder-supervisor lifetime.
    pub runtime_lifetime: RuntimeLifetimeId,
}

/// What `server.stop` ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopScope {
    /// Stop the control server; programs keep running in their holders.
    ControlOnly,
    /// Stop the control server and every program.
    ControlAndPrograms,
}

/// Parameters of `server.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServerStopParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// What the stop ends; there is no default.
    pub scope: StopScope,
}

/// Result of `server.stop` and `server.restart`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ServerTransitionResult {
    /// Boot identity before the transition.
    pub server_boot_id_old: ServerBootId,
    /// Boot identity after a restart.
    pub server_boot_id_new: Option<ServerBootId>,
    /// Program incarnations that survived.
    pub preserved_incarnations: Vec<Incarnation>,
    /// Program incarnations that ended.
    pub ended_incarnations: Vec<Incarnation>,
    /// Operations interrupted by the transition.
    pub interrupted_ops: Vec<OpId>,
}

/// Outbound backlog of one consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ConsumerQueue {
    /// Consumer client.
    pub client_id: ClientId,
    /// Queued outbound bytes.
    pub queue_bytes: ByteCount,
    /// Event delivery lag.
    pub event_lag_ms: DurationMs,
    /// Events dropped and later resynchronized.
    pub resynced_events: u32,
}

/// One recorded recovery fact; reads never clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RecoveryEvidence {
    /// Boot the evidence belongs to.
    pub server_boot_id: ServerBootId,
    /// Restored or relaunched terminal incarnations.
    pub restored: Vec<Incarnation>,
    /// Sequence ranges that could not be recovered.
    pub gaps: Vec<EventSeq>,
    /// Time the evidence was recorded.
    pub recorded_ms: TimestampMs,
}

/// Result of `server.metrics`; reading never clears recovery evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ServerMetricsResult {
    /// Active attach streams.
    pub streams: u32,
    /// Active event subscriptions.
    pub subscriptions: u32,
    /// Per-consumer backlog.
    pub consumers: Vec<ConsumerQueue>,
    /// Latest committed event sequence.
    pub latest_seq: EventSeq,
    /// Recovery evidence of the current runtime lifetime.
    pub recovery: Vec<RecoveryEvidence>,
}

/// State of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OpState {
    /// Admitted, not started.
    Accepted,
    /// In progress.
    Running,
    /// Durably committed.
    Committed,
    /// Failed without effects.
    Failed,
    /// Cancelled before commit.
    Cancelled,
    /// Effects may have begun; the outcome cannot be proven.
    OutcomeUnknown,
}

/// Parameters of `op.get` and `op.cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OpRefParams {
    /// Operation identity.
    pub op_id: OpId,
}

/// Inspectable operation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OpRecord {
    /// Operation identity.
    pub op_id: OpId,
    /// Method that created the operation.
    pub method: MethodName,
    /// Current state.
    pub state: OpenEnum<OpState>,
    /// Method result, once committed.
    pub result: Option<Value>,
    /// Structured error, once failed.
    pub error: Option<ErrorData>,
    /// Sequence of the latest state change.
    pub updated_seq: EventSeq,
}

/// Parameters of `session.snapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SessionSnapshotParams {
    /// Page of the terminal listing.
    pub page: Option<PageRequest>,
}

/// Atomic session snapshot pinned to one revision and sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SessionSnapshotResult {
    /// Snapshot revision.
    pub revision: Revision,
    /// Event sequence the snapshot reflects.
    pub snapshot_seq: EventSeq,
    /// Terminals in this page.
    pub terminals: Vec<TerminalRecord>,
    /// Registered agents.
    pub agents: Vec<AgentBinding>,
    /// Leases.
    pub leases: Vec<LeaseRecord>,
    /// Layout tree at the same revision.
    pub layout: crate::schema::methods::layout::LayoutTree,
    /// Paging facts.
    pub page: PageInfo,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"hello", Query, Untargeted, HelloParams => HelloResult,
    "Negotiate generation, credential and framing; must be first.", [UnsupportedGeneration, InvalidCredential, CredentialExpired]},
    method! {"server.status", Query, Untargeted, NoParams => ServerStatusResult,
    "Report control-server state without starting it.", [StaleEndpoint]},
    method! {"server.start", Mutation, Untargeted, ServerStartParams => ServerStartResult,
    "Start the control server explicitly.", [AlreadyRunning, SpawnFailed]},
    method! {"server.stop", Mutation, Untargeted, ServerStopParams => ServerTransitionResult,
    "Stop the control server with an explicit scope.", [AlreadyStopped]},
    method! {"server.restart", Mutation, Untargeted, ServerStartParams => ServerTransitionResult,
    "Restart the control server; holders keep programs alive.", []},
    method! {"server.metrics", Query, Untargeted, NoParams => ServerMetricsResult,
    "Read per-consumer queues, lag and recovery evidence.", []},
    method! {"op.get", Query, Lookup, OpRefParams => OpRecord,
    "Inspect an operation by its identity.", [OpExpired]},
    method! {"op.cancel", Mutation, Lookup, OpRefParams => OpRecord,
    "Cancel an operation before it commits.", []},
    method! {"session.snapshot", Query, Untargeted, SessionSnapshotParams => SessionSnapshotResult,
    "Read one atomic, paginated snapshot of the session.", [StaleEndpoint]},
];
