//! `agent.*`: declared agents, children, notes and conversations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::conversation_records::{
    AgentRecordsParams, AppendRecordsParams, ConversationTurn, MessagePhase, RecordBlock,
    RecordsAppended, ToolLink,
};
use crate::domain::session_binding::{ConversationCursor, SessionChange};
use crate::errors::OpId;
use crate::schema::common::{PageInfo, PageRequest, Target};
use crate::schema::ids::{HistoryId, NoteId, ProcessToken, RecordId, TerminalId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{AgentBinding, SessionRef, StatusSnapshot};
use crate::schema::scalar::{
    DurationMs, EventSeq, HistoryRevision, Incarnation, OpenEnum, Revision, TimestampMs, TurnNumber,
};

/// Parameters of `agent.register`; the label is arbitrary and never brand-matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentRegisterParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal of the registering program.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Agent label.
    pub label: String,
    /// Provider session, when known.
    pub session: Option<SessionRef>,
}

/// Parameters of `agent.update_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentUpdateSessionParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal of the reporting program.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// New provider session.
    pub session: SessionRef,
    /// Conversation history the session continues.
    pub history_id: Option<HistoryId>,
    /// Why the binding changes; required once the incarnation is bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<SessionChange>,
    /// Record that becomes the active chain head; only with reason `branch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<RecordId>,
}

/// Parameters of `agent.release`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentReleaseParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal of the registered program.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
}

/// Parameters of agent reads that name one terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentTargetParams {
    /// Terminal.
    pub target: Target,
}

/// Result of `agent.get` and agent mutations; null agent means a plain program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentResult {
    /// Registered agent, or null.
    pub agent: Option<AgentBinding>,
    /// Terminal revision.
    pub revision: Revision,
}

/// Parameters of `agent.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentListParams {
    /// Page request.
    pub page: Option<PageRequest>,
}

/// Result of `agent.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentListResult {
    /// Agents.
    pub agents: Vec<AgentBinding>,
    /// Paging facts.
    pub page: PageInfo,
}

/// One child of a parent incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ChildRecord {
    /// Child terminal.
    pub terminal_id: TerminalId,
    /// Child program incarnation.
    pub process_incarnation: Incarnation,
    /// Child's declared status.
    pub status: StatusSnapshot,
    /// Unacknowledged news from this child.
    pub unseen_news: u32,
}

/// Result of `agent.children`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentChildrenResult {
    /// Children.
    pub children: Vec<ChildRecord>,
    /// Revision.
    pub revision: Revision,
}

/// How a note reaches its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoteDelivery {
    /// Typed into the parent when it is neither working nor blocked.
    Typed,
    /// Recorded as an event only.
    Event,
}

/// Parameters of `agent.note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentNoteParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Parent terminal.
    pub target: Target,
    /// Child terminal the note is about.
    pub child: Target,
    /// Note text.
    pub text: String,
    /// Requested delivery.
    pub delivery: NoteDelivery,
}

/// What happened to a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoteOutcome {
    /// Typed into the parent.
    Delivered,
    /// Queued until the parent can take it.
    Queued,
    /// Recorded as an event only.
    EventOnly,
}

/// Result of `agent.note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentNoteResult {
    /// Note identity.
    pub note_id: NoteId,
    /// Outcome.
    pub outcome: OpenEnum<NoteOutcome>,
}

/// When `agent.wait_children` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChildWaitMode {
    /// On the first news from any child.
    Any,
    /// When every child has stopped.
    All,
}

/// Parameters of `agent.wait_children`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentWaitChildrenParams {
    /// Parent terminal.
    pub target: Target,
    /// Whether to return on the first news or when every child stopped.
    pub mode: ChildWaitMode,
    /// Wait bound.
    pub timeout_ms: DurationMs,
}

/// Result of `agent.wait_children`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentWaitChildrenResult {
    /// Children with news.
    pub news: Vec<ChildRecord>,
    /// Whether the bound passed.
    pub timed_out: bool,
    /// Sequence to continue from.
    pub seq: EventSeq,
}

/// Parameters of `agent.conversation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentConversationParams {
    /// Terminal of the agent.
    pub target: Target,
    /// Continuation from a previous page.
    pub page: Option<PageRequest>,
    /// Position bound to a history identity and revision; absent starts the active chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<ConversationCursor>,
}

/// One conversation record with provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ConversationRecord {
    /// Record identity, stable across revisions that keep the record.
    pub record_id: RecordId,
    /// Record this one continues; absent for a chain root.
    pub parent: Option<RecordId>,
    /// Record role, such as user or assistant.
    pub role: String,
    /// Content blocks; reported records carry only typed blocks.
    pub blocks: Vec<RecordBlock>,
    /// Provider store the record came from, or `records` for reported records.
    pub provenance: String,
    /// Phase of assistant text, when the provider recorded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<OpenEnum<MessagePhase>>,
    /// Status turn that was running when the record was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnNumber>,
    /// Time the provider recorded for the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<TimestampMs>,
    /// Records whose calls this record's tool results answer, in block order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ToolLink>,
}

/// Result of `agent.conversation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentConversationResult {
    /// History identity the page belongs to.
    pub history_id: HistoryId,
    /// Observed history revision every record of the page belongs to.
    pub revision: HistoryRevision,
    /// Last record of the chain read; absent for an empty history.
    pub head: Option<RecordId>,
    /// Whether the chain read is the active chain.
    pub active: bool,
    /// Leaf records outside the chain read, addressable through a cursor head.
    pub off_branch: Vec<RecordId>,
    /// Records.
    pub records: Vec<ConversationRecord>,
    /// Paging facts.
    pub page: PageInfo,
    /// Continuation bound to this history identity and revision; absent on the last page.
    pub next_cursor: Option<ConversationCursor>,
    /// Status turns of the page's reported records, in turn order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<ConversationTurn>,
}

/// Parameters of `agent.history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentHistoryParams {
    /// Page request.
    pub page: Option<PageRequest>,
    /// Provider store to list: a relative path below the authorized history root; absent
    /// lists the root. A path that leaves the root is refused as `forbidden`.
    #[serde(default)]
    pub store: Option<String>,
}

/// Result of `agent.history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentHistoryResult {
    /// Sessions found in authorized provider stores.
    pub sessions: Vec<SessionRef>,
    /// Paging facts.
    pub page: PageInfo,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"agent.register", Mutation, Terminal, AgentRegisterParams => AgentResult,
    "Register an agent as the owning process.", [NotOwner, InvalidAgent, AgentConflict]},
    method! {"agent.get", Query, Terminal, AgentTargetParams => AgentResult,
    "Read a terminal's agent; a plain program has a null agent.", []},
    method! {"agent.list", Query, Untargeted, AgentListParams => AgentListResult,
    "List registered agents.", []},
    method! {"agent.update_session", Mutation, Terminal, AgentUpdateSessionParams => AgentResult,
    "Rebind the provider session as the owning process.", [NotOwner, InvalidSession]},
    method! {"agent.release", Mutation, Terminal, AgentReleaseParams => AgentResult,
    "Release the agent binding as the owning process.", [NotOwner]},
    method! {"agent.children", Query, Terminal, AgentTargetParams => AgentChildrenResult,
    "List children of a parent incarnation.", []},
    method! {"agent.note", Mutation, Terminal, AgentNoteParams => AgentNoteResult,
    "Record a note from a child to its parent.", [NotesDisabled, ParentFinished, NotChild, Blocked, NoLeaseForTyped]},
    method! {"agent.wait_children", Query, Terminal, AgentWaitChildrenParams => AgentWaitChildrenResult,
    "Wait for news from children.", [CursorExpired]},
    method! {"agent.conversation", Query, Terminal, AgentConversationParams => AgentConversationResult,
    "Read conversation records with provenance.", [CursorExpired, HistoryChanged, ConversationUnavailable, FrameTooLarge]},
    method! {"agent.append_records", Mutation, Terminal, AppendRecordsParams => RecordsAppended,
    "Append typed conversation records as the owning process.", [NotOwner, InvalidRecords, TooLarge]},
    method! {"agent.records", Query, Terminal, AgentRecordsParams => AgentConversationResult,
    "Read the typed records and turns an agent's owning process reported.", [CursorExpired, HistoryChanged, ConversationUnavailable]},
    method! {"agent.history", Query, Untargeted, AgentHistoryParams => AgentHistoryResult,
    "List sessions found in authorized provider stores.", [StoreUnavailable, PathNotFound, TooLarge]},
];
