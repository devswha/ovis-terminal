//! `status.*`, `prompt.*` and `turn.*`: declared status, versioned prompts and turns.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::agent_status::IngestCounters;
use crate::errors::OpId;
use crate::schema::common::Target;
use crate::schema::ids::{AgentId, LeaseId, ProcessToken, PromptId, TerminalId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{AgentState, SessionRef, StatusReport, StatusSnapshot};
use crate::schema::scalar::{
    DurationMs, EventSeq, FinishGeneration, Incarnation, OpenEnum, RecordVersion, TurnNumber,
};

/// Parameters of `status.get`, `prompt.list` and `turn.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StatusTargetParams {
    /// Terminal.
    pub target: Target,
}

/// Parameters of `status.report`, authorized by the process token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StatusReportParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Reported fields.
    pub record: StatusReport,
    /// Producer sequence; must exceed the current one.
    pub seq: EventSeq,
}

/// Parameters of `status.clear`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StatusClearParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Child record to clear; absent clears the root.
    pub id: Option<String>,
}

/// Result of status reads and reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct StatusResult {
    /// Declared status.
    pub status: StatusSnapshot,
    /// Sequence of the latest status event.
    pub seq: EventSeq,
    /// OSC records this terminal's ingestion dropped.
    pub ingest: IngestCounters,
}

/// Kind of question a prompt asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptKind {
    /// Permission to perform an action.
    Permission,
    /// A free or multiple-choice question.
    Question,
    /// Credentials or authentication.
    Auth,
    /// A structured form.
    Form,
    /// Anything else.
    Other,
}

/// How an answer reaches the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptMechanism {
    /// Encoded as keys through the interact lease.
    Keys,
    /// Delivered as a typed RPC answer.
    Rpc,
    /// Delivered as a typed extension event.
    Extension,
}

/// State of a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptState {
    /// Awaiting an answer.
    Open,
    /// Answered.
    Answered,
    /// Dismissed.
    Dismissed,
    /// Expired with its program incarnation.
    Expired,
}

/// One answer choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptChoice {
    /// Stable choice identity.
    pub id: String,
    /// Display label.
    pub label: String,
}

/// Prompt definition a producer opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptDefinition {
    /// Kind.
    pub kind: PromptKind,
    /// Question text.
    pub text: String,
    /// Choices, for choice prompts.
    pub choices: Vec<PromptChoice>,
    /// Delivery mechanism.
    pub mechanism: PromptMechanism,
}

/// Versioned prompt record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PromptRecord {
    /// Prompt identity.
    pub prompt_id: PromptId,
    /// Version; answers must name it.
    pub version: RecordVersion,
    /// Terminal.
    pub terminal_id: TerminalId,
    /// Program incarnation that opened it.
    pub process_incarnation: Incarnation,
    /// Kind.
    pub kind: OpenEnum<PromptKind>,
    /// Question text.
    pub text: String,
    /// Choices, for choice prompts.
    pub choices: Vec<PromptChoice>,
    /// Delivery mechanism.
    pub mechanism: OpenEnum<PromptMechanism>,
    /// State.
    pub state: OpenEnum<PromptState>,
    /// Sequence of the opening event.
    pub opened_seq: EventSeq,
}

/// Parameters of `prompt.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptOpenParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal.
    pub target: Target,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Definition.
    pub prompt: PromptDefinition,
}

/// Parameters of `prompt.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptListParams {
    /// Terminal; absent lists every terminal's prompts.
    pub target: Option<Target>,
    /// Only prompts in these states.
    pub states: Option<Vec<PromptState>>,
}

/// Result of `prompt.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PromptListResult {
    /// Prompts.
    pub prompts: Vec<PromptRecord>,
}

/// Parameters of `prompt.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptGetParams {
    /// Prompt.
    pub prompt_id: PromptId,
}

/// An answer to one prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum PromptAnswer {
    /// Choice identity.
    Choice(String),
    /// Form field values.
    Fields(BTreeMap<String, String>),
    /// Free text.
    Text(String),
}

/// Parameters of `prompt.answer`, authorized independently of blocked status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptAnswerParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Prompt.
    pub prompt_id: PromptId,
    /// Version the caller answers.
    pub version: RecordVersion,
    /// Answer.
    pub answer: PromptAnswer,
    /// Interact lease, for key-delivered answers.
    pub lease_id: Option<LeaseId>,
}

/// How a prompt closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptCloseOutcome {
    /// Answered.
    Answered,
    /// Dismissed.
    Dismissed,
    /// Expired.
    Expired,
}

/// Parameters of `prompt.close`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromptCloseParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Prompt.
    pub prompt_id: PromptId,
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Outcome.
    pub outcome: PromptCloseOutcome,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnOutcome {
    /// Completed normally.
    Completed,
    /// Ended blocked on a prompt.
    Blocked,
    /// The program exited.
    Exited,
    /// The terminal was lost.
    Lost,
    /// Cancelled.
    Cancelled,
    /// The wait bound passed; no completion is implied.
    Timeout,
    /// Ended on an error report.
    Failed,
}

/// Turn facts of a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TurnRecord {
    /// Turn number.
    pub turn: TurnNumber,
    /// Outcome, once ended.
    pub outcome: Option<OpenEnum<TurnOutcome>>,
    /// Sequence that started the turn.
    pub started_seq: Option<EventSeq>,
    /// Sequence that ended the turn.
    pub ended_seq: Option<EventSeq>,
    /// Send operation the turn answers, when known.
    pub send_op: Option<OpId>,
    /// Finish generation this turn's end produced.
    pub finish_generation: Option<FinishGeneration>,
    /// Root state the turn ended on.
    pub ended_on: Option<OpenEnum<AgentState>>,
    /// Program incarnation that ran the turn.
    pub process_incarnation: Option<Incarnation>,
    /// Registered agent that ran the turn.
    pub agent_id: Option<AgentId>,
    /// Provider session of that agent.
    pub session: Option<SessionRef>,
}

/// Parameters of `turn.wait`, bound to a send or a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TurnWaitParams {
    /// Terminal.
    pub target: Target,
    /// Send operation whose turn to wait for.
    pub send_op: Option<OpId>,
    /// Turn to wait for.
    pub turn: Option<TurnNumber>,
    /// Wait bound.
    pub timeout_ms: DurationMs,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"status.get", Query, Terminal, StatusTargetParams => StatusResult,
    "Read declared status.", []},
    method! {"status.report", Mutation, Terminal, StatusReportParams => StatusResult,
    "Report status as the owning process; stale sequences are rejected.", [StaleSeq, NotOwner, InvalidRecord]},
    method! {"status.clear", Mutation, Terminal, StatusClearParams => StatusResult,
    "Clear a status record as the owning process.", [NotOwner]},
    method! {"prompt.list", Query, Untargeted, PromptListParams => PromptListResult,
    "List prompts.", [AmbiguousTarget, StaleTarget]},
    method! {"prompt.get", Query, Lookup, PromptGetParams => PromptRecord,
    "Read one prompt.", []},
    method! {"prompt.open", Mutation, Terminal, PromptOpenParams => PromptRecord,
    "Open a typed prompt as the owning process.", [NotOwner, InvalidPrompt, LimitExceeded]},
    method! {"prompt.answer", Mutation, Lookup, PromptAnswerParams => PromptRecord,
    "Answer a prompt version with independent authorization.", [VersionConflict, PromptClosed, AnswerInvalid, DeliveryUnsupported, NotAuthorized, NoLease]},
    method! {"prompt.close", Mutation, Lookup, PromptCloseParams => PromptRecord,
    "Close a prompt as the owning process.", [NotOwner, AlreadyClosed]},
    method! {"turn.get", Query, Terminal, StatusTargetParams => TurnRecord,
    "Read the current turn.", []},
    method! {"turn.wait", Query, Terminal, TurnWaitParams => TurnRecord,
    "Wait for the turn of a send; timeout never implies completion.", [UnknownTurn, CursorExpired]},
];
