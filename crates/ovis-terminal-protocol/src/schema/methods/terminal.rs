//! `terminal.*` lifecycle: launch, list, inspect, kill, remove, keep, recover, label, rename.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::template_launch::FirstMessageRecord;
use crate::errors::OpId;
use crate::schema::common::{Expect, Geometry, MachineRef, PageInfo, PageRequest, Target};
use crate::schema::ids::{TemplateName, TerminalId, TerminalName};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{ParentRef, ProcessRecord, TerminalRecord, TerminalState};
use crate::schema::scalar::{DurationMs, EventSeq, Incarnation, OpenEnum, Revision};

/// Program a terminal launches: an explicit argument vector or a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum LaunchProgram {
    /// Argument vector; never interpreted by a shell.
    Argv(Vec<String>),
    /// Versioned template by name.
    Template {
        /// Template name.
        name: TemplateName,
        /// Template version; absent selects the latest.
        version: Option<String>,
        /// Task text passed as data, never interpolated into a command line.
        task: Option<String>,
    },
}

/// Versioned template selected by stable name; the same launch contract as an argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TemplateSelector {
    /// Template name.
    pub name: TemplateName,
    /// Template version; absent selects the latest.
    pub version: Option<String>,
    /// Task text passed as data, never interpolated into a command line.
    pub task: Option<String>,
}

/// Parameters of `terminal.run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalRunParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Program to launch; exactly one of `program` and `template` is required.
    pub program: Option<LaunchProgram>,
    /// Template to launch; exactly one of `program` and `template` is required.
    pub template: Option<TemplateSelector>,
    /// Initial grid size.
    pub geometry: Geometry,
    /// Working directory.
    pub cwd: Option<String>,
    /// Environment additions.
    pub env: Option<BTreeMap<String, String>>,
    /// Unique name.
    pub name: Option<TerminalName>,
    /// Labels.
    pub labels: Option<BTreeMap<String, String>>,
    /// Parent binding by incarnation.
    pub parent: Option<ParentRef>,
    /// Policy toward the parent.
    pub child_policy: Option<ChildPolicy>,
    /// Keep the terminal after its program dies.
    pub keep: Option<bool>,
    /// First message released only on declared readiness.
    pub first_message: Option<FirstMessage>,
    /// Machine to launch on.
    pub machine: Option<MachineRef>,
}

/// First message queued under the launch and released only on declared readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct FirstMessage {
    /// Literal text; submitted after the agent declares readiness.
    pub text: String,
    /// Expire the message if readiness is not declared in time; absent means 30 s.
    pub deadline_ms: Option<DurationMs>,
}

/// How a child relates to its parent incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChildPolicy {
    /// Deliver child news to the parent as notes.
    pub notify: bool,
    /// End this child when the parent incarnation finishes.
    pub die_with_parent: bool,
}

/// Spawn outcome reported separately from creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SpawnState {
    /// The holder confirmed the spawn.
    Spawned,
    /// The spawn failed; the terminal record still exists.
    Failed,
    /// The holder has not answered yet; watch `terminal.spawned`.
    Pending,
}

/// Result of `terminal.run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalRunResult {
    /// Created terminal.
    pub terminal: TerminalRecord,
    /// Spawn outcome.
    pub spawn: OpenEnum<SpawnState>,
    /// Sequence of the creation event.
    pub created_seq: EventSeq,
    /// First message queued under this launch, when one was given.
    pub first_message: Option<FirstMessageRecord>,
}

/// Parameters of `terminal.first_message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalFirstMessageParams {
    /// Operation that launched the terminal.
    pub launch_op: OpId,
    /// Wait up to this long for a final state before answering the current one.
    pub settled_within_ms: Option<DurationMs>,
}

/// Parameters of `terminal.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalListParams {
    /// Only terminals in these states.
    pub states: Option<Vec<TerminalState>>,
    /// Only terminals carrying all of these labels.
    pub labels: Option<BTreeMap<String, String>>,
    /// Only children of this terminal.
    pub children_of: Option<TerminalId>,
    /// Only terminals on this machine.
    pub machine: Option<MachineRef>,
    /// Page request.
    pub page: Option<PageRequest>,
}

/// Result of `terminal.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalListResult {
    /// Terminals in this page.
    pub terminals: Vec<TerminalRecord>,
    /// Paging facts.
    pub page: PageInfo,
    /// Oldest finished terminal still retained.
    pub retention_floor: EventSeq,
}

/// Parameters of methods that only name a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalGetParams {
    /// Terminal to read.
    pub target: Target,
}

/// Escalation schedule of `terminal.kill`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct KillEscalation {
    /// Wait after hangup before terminate.
    pub hup_ms: DurationMs,
    /// Wait after terminate before kill.
    pub term_ms: DurationMs,
}

/// Parameters of `terminal.kill`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalKillParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to kill.
    pub target: Target,
    /// Identities that must still match.
    pub expect: Option<Expect>,
    /// Escalation schedule.
    pub escalation: KillEscalation,
}

/// Observed process outcome after an action was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProcessObservation {
    /// The program exited.
    Exited,
    /// The program still runs.
    StillRunning,
    /// No evidence either way.
    Unknown,
}

/// Result of `terminal.kill`; delivery and observed exit are separate facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalKillResult {
    /// Program incarnation the action was delivered to.
    pub incarnation: Incarnation,
    /// Observed outcome.
    pub observed: OpenEnum<ProcessObservation>,
}

/// Parameters of mutations that only name a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalMutateParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to change.
    pub target: Target,
}

/// Parameters of `terminal.keep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalKeepParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to change.
    pub target: Target,
    /// Keep the terminal after its program dies.
    pub keep: bool,
}

/// Parameters of `terminal.label`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalLabelParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to change.
    pub target: Target,
    /// Labels to set.
    pub set: Option<BTreeMap<String, String>>,
    /// Label keys to remove.
    pub remove: Option<Vec<String>>,
}

/// Parameters of `terminal.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalRenameParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to change.
    pub target: Target,
    /// New name; null clears it.
    pub name: Option<TerminalName>,
}

/// Result of a terminal mutation: the committed record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalMutateResult {
    /// Terminal after the mutation.
    pub terminal: TerminalRecord,
    /// Revision of the commit.
    pub revision: Revision,
    /// Sequence of the commit event.
    pub seq: EventSeq,
}

/// Incarnations `terminal.process_info` must still find; a mismatch reports nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProcessExpect {
    /// Expected terminal incarnation; a mismatch is `stale_target`.
    pub terminal_incarnation: Option<Incarnation>,
    /// Expected program incarnation; a mismatch is `process_replaced`.
    pub process_incarnation: Option<Incarnation>,
}

/// Parameters of `terminal.process_info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalProcessInfoParams {
    /// Terminal to inspect; there is no focused-terminal fallback.
    pub target: Target,
    /// Incarnations the caller holds.
    pub expect: Option<ProcessExpect>,
}

/// Result of `terminal.process_info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalProcessInfoResult {
    /// Terminal the facts belong to.
    pub terminal_id: TerminalId,
    /// Program the terminal launched.
    pub launcher: ProcessRecord,
    /// The launcher record while the launcher holds the foreground; `roles.foreground`
    /// lists every foreground process.
    pub foreground: Option<ProcessRecord>,
    /// Fields this platform can report.
    pub supported_fields: Vec<String>,
    /// Shell, launcher, foreground and background processes, each observed or
    /// explicitly unsupported.
    pub roles: crate::domain::process_info::ProcessRoles,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"terminal.run", Mutation, Untargeted, TerminalRunParams => TerminalRunResult,
    "Launch a program in a new terminal.", [SpawnFailed, DuplicateName, TemplateNotFound, TemplateInvalid, ParentNotFound, ParentFinished, MachineUnreachable, InvalidGeometry, LimitExceeded]},
    method! {"terminal.first_message", Query, Untargeted, TerminalFirstMessageParams => FirstMessageRecord,
    "Read the first message queued under a launch by its requesting principal.", []},
    method! {"terminal.list", Query, Untargeted, TerminalListParams => TerminalListResult,
    "List terminals at one pinned revision.", [MachineUnreachable]},
    method! {"terminal.get", Query, Terminal, TerminalGetParams => TerminalRecord,
    "Read one terminal record.", []},
    method! {"terminal.kill", Mutation, Terminal, TerminalKillParams => TerminalKillResult,
    "Signal a program with an explicit escalation schedule.", [AlreadyFinished]},
    method! {"terminal.rm", Mutation, Terminal, TerminalMutateParams => TerminalMutateResult,
    "Remove a finished terminal, leaving a tombstone.", [StillLive, HasChildren]},
    method! {"terminal.keep", Mutation, Terminal, TerminalKeepParams => TerminalMutateResult,
    "Set whether a terminal is kept after its program dies.", []},
    method! {"terminal.recover", Mutation, Terminal, TerminalMutateParams => TerminalMutateResult,
    "Relaunch a lost terminal's program.", [NotLost, RecoveryDisabled, SpawnFailed]},
    method! {"terminal.label", Mutation, Terminal, TerminalLabelParams => TerminalMutateResult,
    "Set or remove terminal labels.", [LimitExceeded]},
    method! {"terminal.rename", Mutation, Terminal, TerminalRenameParams => TerminalMutateResult,
    "Set or clear a terminal's unique name.", [DuplicateName, InvalidName]},
    method! {"terminal.process_info", Query, Terminal, TerminalProcessInfoParams => TerminalProcessInfoResult,
    "Read shell, launcher, foreground and background process facts.", [UnsupportedField, ProcessReplaced]},
];
