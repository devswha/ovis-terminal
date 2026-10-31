//! Advertised limits and capabilities, served by `hello` and `schema.capabilities`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::scalar::{ByteCount, Count, DurationMs, OpenEnum};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

/// Per-object and session-wide bounds; admission budgets, not preallocations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Limits {
    /// Largest frame payload and NDJSON control record.
    pub max_frame_bytes: ByteCount,
    /// Largest DATA chunk on an output stream.
    pub stream_chunk_bytes: ByteCount,
    /// Output credit granted when a stream opens.
    pub credit_initial_bytes: ByteCount,
    /// Largest outstanding output credit.
    pub credit_max_bytes: ByteCount,
    /// Per-stream backlog before a gap and checkpoint replace it.
    pub stream_backlog_bytes: ByteCount,
    /// Accepted input queued for one terminal.
    pub input_queue_bytes: ByteCount,
    /// Retained events available for replay.
    pub event_replay_events: Count,
    /// Retained event bytes available for replay.
    pub event_replay_bytes: ByteCount,
    /// Idle interval after which a heartbeat is sent.
    pub heartbeat_ms: DurationMs,
    /// Deadline applied when a request names none.
    pub rpc_default_deadline_ms: DurationMs,
    /// How long an operation result stays retrievable by its id.
    pub op_retention_ms: DurationMs,
    /// Largest document value.
    pub doc_max_bytes: ByteCount,
    /// Retained history lines per terminal.
    pub history_lines: Count,
    /// Retained raw output per terminal.
    pub output_ring_bytes: ByteCount,
    /// Requests in flight on one connection.
    pub max_in_flight_requests: Count,
    /// Encoded bytes queued for one connection but not yet written; past it the
    /// connection closes with `$/closing{reason: backpressure}`.
    pub connection_unwritten_bytes: ByteCount,
    /// Session budget for runtime plus holder resident memory.
    pub session_rss_bytes: ByteCount,
    /// Session budget for retained data, evidence excluded.
    pub session_retained_bytes: ByteCount,
    /// Session budget for accepted input across terminals.
    pub session_input_bytes: ByteCount,
    /// Session budget for stream backlog references.
    pub session_backlog_bytes: ByteCount,
    /// Session budget for event queues.
    pub session_event_queue_bytes: ByteCount,
    /// Session budget for resident VT grids and history.
    pub session_vt_bytes: ByteCount,
}

impl Limits {
    /// Returns the settled v1 defaults.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            max_frame_bytes: ByteCount::new(16 * MIB),
            stream_chunk_bytes: ByteCount::new(16 * KIB),
            credit_initial_bytes: ByteCount::new(256 * KIB),
            credit_max_bytes: ByteCount::new(MIB),
            stream_backlog_bytes: ByteCount::new(4 * MIB),
            input_queue_bytes: ByteCount::new(16 * MIB),
            event_replay_events: Count::new(10_000),
            event_replay_bytes: ByteCount::new(64 * MIB),
            heartbeat_ms: DurationMs::new(15_000),
            rpc_default_deadline_ms: DurationMs::new(10_000),
            op_retention_ms: DurationMs::new(86_400_000),
            doc_max_bytes: ByteCount::new(MIB),
            history_lines: Count::new(100_000),
            output_ring_bytes: ByteCount::new(64 * MIB),
            max_in_flight_requests: Count::new(64),
            connection_unwritten_bytes: ByteCount::new(64 * MIB),
            session_rss_bytes: ByteCount::new(GIB),
            session_retained_bytes: ByteCount::new(4 * GIB),
            session_input_bytes: ByteCount::new(64 * MIB),
            session_backlog_bytes: ByteCount::new(64 * MIB),
            session_event_queue_bytes: ByteCount::new(64 * MIB),
            session_vt_bytes: ByteCount::new(128 * MIB),
        }
    }
}

/// Transport a runtime endpoint can speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransportKind {
    /// Unix domain socket.
    Unix,
    /// Windows named pipe.
    Pipe,
    /// Standard input and output, for SSH and custom transports.
    Stdio,
    /// Authenticated WebSocket; one binary message per frame.
    Websocket,
}

/// How an attach stream delivers terminal bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamMode {
    /// DATA frames on a dedicated channel of a framed connection.
    Channel,
    /// Base64 notifications on an NDJSON connection.
    Plain,
}

/// Features this runtime serves; a client never assumes an absent entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Capabilities {
    /// Transports this endpoint serves.
    pub transports: Vec<OpenEnum<TransportKind>>,
    /// Attach stream delivery modes.
    pub stream_modes: Vec<OpenEnum<StreamMode>>,
    /// Whether `hello` may upgrade the connection to framed mode.
    pub framed: bool,
}
