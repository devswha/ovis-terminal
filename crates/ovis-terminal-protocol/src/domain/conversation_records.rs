//! Typed conversation records and turn records (plan todo 37).
//!
//! An agent's owning process reports what was said and done as typed records: user,
//! assistant and tool records whose blocks are text, tool calls and tool results. The
//! runtime keeps their order and identities, links every tool result to the record that
//! made its call, and stamps each record with the status turn that was running when it
//! arrived. Provider stores are parsed by their clients; terminal output is never a
//! source of records. `agent.records` serves them outside the operation ledger, and an
//! append's receipt names what it stored by identity, count and digest, never by body.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::session_binding::ConversationCursor;
use crate::errors::OpId;
use crate::schema::common::{PageRequest, Target};
use crate::schema::ids::{AgentId, CallId, HistoryId, ProcessToken, RecordId};
use crate::schema::methods::status::TurnOutcome;
use crate::schema::scalar::{
    ByteCount, EventSeq, HistoryRevision, Incarnation, OpenEnum, RecordCount, RecordPosition,
    TimestampMs, TurnNumber,
};

/// The structured source a conversation read consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConversationSource {
    /// Typed records the owning process reported with `agent.append_records`.
    Records,
}

/// Parameters of `agent.records`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentRecordsParams {
    /// Terminal of the agent.
    pub target: Target,
    /// Continuation from a previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageRequest>,
    /// Position bound to the records' identity and revision; absent starts at the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<ConversationCursor>,
}

/// `sha256:` and the lowercase hex SHA-256 of the appended records' stored encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RecordsDigest(String);

impl RecordsDigest {
    /// Wraps a `sha256:<hex>` digest.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who a record speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecordRole {
    /// The person driving the agent; text and tool results.
    User,
    /// The agent; text and tool calls.
    Assistant,
    /// A tool's own record; tool results only.
    Tool,
}

/// Whether assistant text is progress commentary or the turn's final answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessagePhase {
    /// Progress prose written while the turn continues.
    Commentary,
    /// The answer that closes the turn.
    Final,
}

/// One typed content block of a record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ContentBlock {
    /// Plain text.
    Text {
        /// The text.
        text: String,
    },
    /// A tool invocation the assistant made.
    ToolCall {
        /// Call identity, unique within the agent's records.
        call_id: CallId,
        /// Tool name as the provider reported it.
        name: String,
        /// Tool input as the provider reported it.
        input: Value,
    },
    /// The result of an earlier tool call.
    ToolResult {
        /// Identity of the call this result answers.
        call_id: CallId,
        /// Result text.
        output: String,
        /// Whether the tool reported a failure.
        is_error: bool,
    },
}

/// A record block: typed, or provider content a transcript carried as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[non_exhaustive]
pub enum RecordBlock {
    /// A block of the typed record contract; every reported record carries only these.
    Typed(ContentBlock),
    /// A transcript block outside the typed contract, unchanged.
    Provider(Value),
}

/// The record that made the call a tool result answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ToolLink {
    /// The answered call.
    pub call_id: CallId,
    /// The record carrying the call.
    pub call_record: RecordId,
}

impl ToolLink {
    /// Links `call_id` to the record `call_record` that made it.
    #[must_use]
    pub const fn new(call_id: CallId, call_record: RecordId) -> Self {
        Self {
            call_id,
            call_record,
        }
    }
}

/// One record as the owning process reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RecordInput {
    /// Record identity, unique within the agent's records.
    pub record_id: RecordId,
    /// Who the record speaks for.
    pub role: RecordRole,
    /// Phase of assistant text, when the provider recorded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<MessagePhase>,
    /// Content blocks in provider order, text and tool results side by side.
    pub blocks: Vec<ContentBlock>,
    /// Time the provider recorded for the record, when it recorded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<TimestampMs>,
}

/// Model and reasoning metadata the provider recorded for the running turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TurnMetadata {
    /// Model name as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning effort as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Parameters of `agent.append_records`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AppendRecordsParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal of the reporting program.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Records to append, in order.
    #[serde(default)]
    pub records: Vec<RecordInput>,
    /// Metadata of the turn the status root is working on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnMetadata>,
}

/// Whether a turn is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnState {
    /// The status root is still working on the turn.
    Open,
    /// The status root left working; outcome and end are known only when recorded.
    Ended,
}

/// One status turn as conversation records see it.
///
/// Boundaries, outcome and times come from the agent status turn; model and reasoning
/// are what the provider recorded. Every unknown fact is null, never estimated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ConversationTurn {
    /// Status turn number.
    pub turn: TurnNumber,
    /// Whether the turn still runs.
    pub state: OpenEnum<TurnState>,
    /// How the turn ended, once known.
    pub outcome: Option<OpenEnum<TurnOutcome>>,
    /// Status event that started the turn.
    pub started_seq: Option<EventSeq>,
    /// Status event that ended the turn.
    pub ended_seq: Option<EventSeq>,
    /// Acceptance time of the report that started the turn.
    pub started_ms: Option<TimestampMs>,
    /// Acceptance time of the report that ended the turn.
    pub ended_ms: Option<TimestampMs>,
    /// Model the provider recorded.
    pub model: Option<String>,
    /// Reasoning effort the provider recorded.
    pub reasoning: Option<String>,
}

impl ConversationTurn {
    /// Turn `turn` in `state` with no recorded facts.
    #[must_use]
    pub const fn new(turn: TurnNumber, state: TurnState) -> Self {
        Self {
            turn,
            state: OpenEnum::Known(state),
            outcome: None,
            started_seq: None,
            ended_seq: None,
            started_ms: None,
            ended_ms: None,
            model: None,
            reasoning: None,
        }
    }
}

/// Payload of `agent.records_appended` and result of `agent.append_records`.
///
/// It names the appended records by identity, count and digest; their bodies are read
/// with `agent.records`, so the receipt stays small whatever the records hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RecordsAppended {
    /// Agent whose records grew.
    pub agent_id: AgentId,
    /// Program incarnation that reported them.
    pub process_incarnation: Incarnation,
    /// Identity of the agent's record history.
    pub history_id: HistoryId,
    /// History revision after the append.
    pub revision: HistoryRevision,
    /// Identities of the appended records, in order.
    pub record_ids: Vec<RecordId>,
    /// Position of the first appended record.
    pub first_position: RecordPosition,
    /// Records the agent holds after the append.
    pub total_records: RecordCount,
    /// Stored bytes of the agent's records after the append.
    pub total_bytes: ByteCount,
    /// `sha256:` over the stored encodings of the appended records, concatenated in order;
    /// the wire reply of `agent.records` is a different serialization.
    pub digest: RecordsDigest,
    /// Turns the appended records or metadata belong to.
    pub turns: Vec<ConversationTurn>,
}

/// Every field of a [`RecordsAppended`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "the agent crate builds append records from every field; a new field must fail its build"
)]
pub struct AppendFacts {
    /// Agent whose records grew.
    pub agent_id: AgentId,
    /// Reporting program incarnation.
    pub process_incarnation: Incarnation,
    /// Record history identity.
    pub history_id: HistoryId,
    /// Revision after the append.
    pub revision: HistoryRevision,
    /// Appended record identities.
    pub record_ids: Vec<RecordId>,
    /// Position of the first.
    pub first_position: RecordPosition,
    /// Records held after the append.
    pub total_records: RecordCount,
    /// Stored bytes held after the append.
    pub total_bytes: ByteCount,
    /// Digest of the appended records.
    pub digest: RecordsDigest,
    /// Their turns.
    pub turns: Vec<ConversationTurn>,
}

impl From<AppendFacts> for RecordsAppended {
    fn from(facts: AppendFacts) -> Self {
        Self {
            agent_id: facts.agent_id,
            process_incarnation: facts.process_incarnation,
            history_id: facts.history_id,
            revision: facts.revision,
            record_ids: facts.record_ids,
            first_position: facts.first_position,
            total_records: facts.total_records,
            total_bytes: facts.total_bytes,
            digest: facts.digest,
            turns: facts.turns,
        }
    }
}

mod unavailable;

pub use unavailable::{Provenance, Unavailable, UnavailableReason};

#[cfg(test)]
mod tests;
