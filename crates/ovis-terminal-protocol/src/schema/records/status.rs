//! Declared status and agent binding records.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::agent_status::OscProvenance;
use crate::schema::ids::{AgentId, HistoryId};
use crate::schema::methods::status::TurnRecord;
use crate::schema::scalar::{
    EventSeq, FinishGeneration, Incarnation, OpenEnum, ReportSeq, Revision, TimestampMs, TurnNumber,
};

/// Lifecycle state an agent declares; later releases may add values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentState {
    /// Working on a turn.
    Working,
    /// Waiting for input.
    Idle,
    /// Blocked on a prompt.
    Blocked,
    /// Finished its task.
    Done,
    /// Working without holding the foreground.
    Background,
    /// Stopped on an error.
    Error,
}

/// Where an accepted status report came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StatusOrigin {
    /// An OSC 7501 sequence in the program's output.
    Osc,
    /// A `status.report` call by the owning process.
    Rpc,
}

/// Status fields a producer reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StatusReport {
    /// Declared state; unknown values pass through.
    pub state: OpenEnum<AgentState>,
    /// Record identity for child records; absent for the root.
    pub id: Option<String>,
    /// Producer-defined kind.
    pub kind: Option<String>,
    /// Progress percentage.
    pub progress: Option<u8>,
    /// Producer application label.
    pub app: Option<String>,
    /// Title text.
    pub title: Option<String>,
    /// Message text.
    pub msg: Option<String>,
}

/// One accepted status record with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct StatusEntry {
    /// Reported fields.
    pub report: StatusReport,
    /// Where the report came from.
    pub origin: OpenEnum<StatusOrigin>,
    /// Raw OSC payload, when the origin is OSC.
    pub raw: Option<String>,
    /// Field decoding failures; decoded text is never invented.
    pub decode_errors: Vec<String>,
    /// Sequence of the accepting event.
    pub seq: EventSeq,
    /// Time the state was entered.
    pub since_ms: TimestampMs,
    /// Producer status sequence that was accepted.
    pub report_seq: ReportSeq,
    /// Turn current when the report was accepted.
    pub turn: TurnNumber,
    /// Raw fields and decoding outcome, when the origin is OSC.
    pub osc: Option<OscProvenance>,
}

/// Declared status of a terminal; `reported: false` is the only never-reported form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct StatusSnapshot {
    /// Whether any report was ever accepted for this program incarnation.
    pub reported: bool,
    /// Root record, when reported.
    pub root: Option<StatusEntry>,
    /// Child records keyed by record identity.
    pub records: BTreeMap<String, StatusEntry>,
    /// Current turn number.
    pub turn: TurnNumber,
    /// Program incarnation the status belongs to.
    pub process_incarnation: Option<Incarnation>,
    /// Highest accepted producer sequence of this incarnation.
    pub report_seq: Option<ReportSeq>,
    /// Number of turns that finished; viewing never changes it.
    pub finish_generation: FinishGeneration,
    /// The most recently finished turn.
    pub last_turn: Option<TurnRecord>,
    /// Registered agent, when one is bound to this incarnation.
    pub agent_id: Option<AgentId>,
    /// Provider session of the registered agent.
    pub session: Option<SessionRef>,
}

/// Provider session an agent is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SessionRef {
    /// Provider store that owns the session.
    pub source: String,
    /// Kind of session identifier.
    pub kind: String,
    /// Opaque session identifier.
    pub value: String,
}

/// Agent registered on a terminal by its owning process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentBinding {
    /// Agent identity.
    pub agent_id: AgentId,
    /// Arbitrary agent label; never matched against brand lists.
    pub label: String,
    /// Program incarnation the binding belongs to.
    pub process_incarnation: Incarnation,
    /// Provider session, when declared.
    pub session: Option<SessionRef>,
    /// Conversation history, when bound.
    pub history_id: Option<HistoryId>,
    /// Binding revision.
    pub revision: Revision,
}
