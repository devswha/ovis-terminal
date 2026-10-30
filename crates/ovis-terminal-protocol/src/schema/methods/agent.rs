//! `agent.*`: declared agents, children, notes and conversations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::OpId;
use crate::schema::common::{PageInfo, PageRequest, Target};
use crate::schema::ids::{HistoryId, NoteId, ProcessToken, TerminalId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{AgentBinding, SessionRef, StatusSnapshot};
use crate::schema::scalar::{DurationMs, EventSeq, Incarnation, OpenEnum, Revision};

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
}

/// One conversation record with provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ConversationRecord {
    /// Record role, such as user or assistant.
    pub role: String,
    /// Typed block content as read from the provider store.
    pub blocks: Vec<Value>,
    /// Provider store the record came from.
    pub provenance: String,
}

/// Result of `agent.conversation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AgentConversationResult {
    /// History identity the page belongs to.
    pub history_id: HistoryId,
    /// Records.
    pub records: Vec<ConversationRecord>,
    /// Paging facts.
    pub page: PageInfo,
}

/// Parameters of `agent.history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentHistoryParams {
    /// Page request.
    pub page: Option<PageRequest>,
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
    method! {"agent.history", Query, Untargeted, AgentHistoryParams => AgentHistoryResult,
    "List sessions found in authorized provider stores.", [StoreUnavailable]},
];
