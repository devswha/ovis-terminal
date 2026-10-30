//! `input.*`: ordered, leased, admission-receipted writes to a terminal.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::common::{Expect, OutputCursor, Target};
use crate::schema::ids::LeaseId;
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::StatusSnapshot;
use crate::schema::scalar::{ByteCount, ByteOffset, DurationMs, EventSeq, OpenEnum, Revision};

/// What follows literal text in the same queue entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Submit {
    /// Nothing; the text is typed but not submitted.
    None,
    /// The Enter key.
    Enter,
    /// A sequence of named keys.
    Keys(Vec<String>),
}

/// Parameters of `input.send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InputSendParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to write to.
    pub target: Target,
    /// Identities that must still match at admission and at the write.
    pub expect: Option<Expect>,
    /// Literal text.
    pub text: String,
    /// Submission appended atomically after the text.
    pub submit: Submit,
    /// Interact lease, required whenever any lease exists.
    pub lease_id: Option<LeaseId>,
}

/// Admission receipt; delivery and agent facts arrive later through `op.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct InputReceipt {
    /// Operation identity.
    pub op_id: OpId,
    /// Position in the terminal's input sequence.
    pub queued_seq: EventSeq,
    /// Bytes admitted.
    pub admitted_bytes: ByteCount,
    /// Output cursor captured atomically with admission.
    pub output_cursor: OutputCursor,
    /// Mode revision the encoding was checked against.
    pub mode_revision: Revision,
}

/// Result of `input.send`, `input.keys`, `input.raw` and `input.paste`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct InputResult {
    /// Admission receipt.
    pub receipt: InputReceipt,
    /// Declared status at admission.
    pub status_at_send: StatusSnapshot,
}

/// Parameters of `input.keys`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InputKeysParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to write to.
    pub target: Target,
    /// Identities that must still match.
    pub expect: Option<Expect>,
    /// Named keys, encoded for the current terminal modes.
    pub keys: Vec<String>,
    /// Interact lease, required whenever any lease exists.
    pub lease_id: Option<LeaseId>,
}

/// Parameters of `input.raw`; raw bytes need their own explicit grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InputRawParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to write to.
    pub target: Target,
    /// Identities that must still match.
    pub expect: Option<Expect>,
    /// Raw bytes, base64-encoded.
    pub data_b64: String,
    /// Interact lease.
    pub lease_id: LeaseId,
}

/// Bracketed-paste policy of `input.paste`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BracketedPolicy {
    /// Bracket when the program enabled bracketed paste.
    Auto,
    /// Refuse unless bracketed paste is enabled.
    Require,
    /// Never bracket.
    Never,
}

/// Parameters of `input.paste`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InputPasteParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to write to.
    pub target: Target,
    /// Identities that must still match.
    pub expect: Option<Expect>,
    /// Text to paste.
    pub text: String,
    /// Bracketing policy.
    pub bracketed: BracketedPolicy,
    /// Interact lease, required whenever any lease exists.
    pub lease_id: Option<LeaseId>,
}

/// Parameters of `input.interrupt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InputInterruptParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Terminal to interrupt.
    pub target: Target,
    /// Identities that must still match.
    pub expect: Option<Expect>,
    /// Key to deliver; absent uses the program's declared stop key.
    pub key: Option<String>,
    /// How long to observe the outcome.
    pub observe_ms: DurationMs,
    /// Interact lease, required whenever any lease exists.
    pub lease_id: Option<LeaseId>,
}

/// Turn outcome observed after an interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnObservation {
    /// The turn ended.
    Ended,
    /// The program is still working.
    Working,
    /// No evidence either way.
    Unknown,
}

/// Result of `input.interrupt`; delivery and observed stop are separate facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct InputInterruptResult {
    /// Admission receipt of the delivered key.
    pub receipt: InputReceipt,
    /// Observed turn outcome.
    pub turn: OpenEnum<TurnObservation>,
    /// Whether an exit or an authenticated stop report proves the program stopped.
    pub stopped: bool,
}

/// Parameters of `input.barrier`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InputBarrierParams {
    /// Terminal to inspect.
    pub target: Target,
    /// Input sequence position to wait for.
    pub queued_seq: EventSeq,
}

/// Result of `input.barrier`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct InputBarrierResult {
    /// Whether every byte up to the position reached the program.
    pub reached: bool,
    /// Raw output offset when the barrier was evaluated.
    pub output_offset: ByteOffset,
    /// Admitted bytes not yet written.
    pub pending_bytes: ByteCount,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"input.send", Mutation, Terminal, InputSendParams => InputResult,
    "Admit literal text with an atomic submission.", [NoLease, LeaseRevoked, Blocked, NotReady, Recovering, Finished, InputOverflow, InvalidEncoding, ProcessReplaced, PasteRefused, ModeMismatch]},
    method! {"input.keys", Mutation, Terminal, InputKeysParams => InputResult,
    "Admit named keys encoded for the current modes.", [NoLease, LeaseRevoked, Blocked, Recovering, Finished, InputOverflow, ProcessReplaced, ModeMismatch, UnknownKey]},
    method! {"input.raw", Mutation, Terminal, InputRawParams => InputResult,
    "Admit raw bytes under an explicit raw grant.", [NoLease, LeaseRevoked, Recovering, Finished, InputOverflow, ProcessReplaced, RawNotAuthorized]},
    method! {"input.paste", Mutation, Terminal, InputPasteParams => InputResult,
    "Admit pasted text with an explicit bracketing policy.", [NoLease, LeaseRevoked, Blocked, Recovering, Finished, InputOverflow, ProcessReplaced, PasteRefused, ModeMismatch]},
    method! {"input.interrupt", Mutation, Terminal, InputInterruptParams => InputInterruptResult,
    "Deliver an interrupt and report the observed outcome separately.", [NoLease, LeaseRevoked, Recovering, Finished, ProcessReplaced]},
    method! {"input.barrier", Query, Terminal, InputBarrierParams => InputBarrierResult,
    "Report whether admitted input reached the program.", []},
];
