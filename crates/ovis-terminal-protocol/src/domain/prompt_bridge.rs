//! Typed records of the in-process prompt bridge (plan todo 40).
//!
//! A program whose prompt widget takes no terminal keys opens it with
//! `mechanism: extension`. Each accepted answer reaches the program's extension as one
//! [`ExtensionEvent`], which the extension acknowledges by the version the answer
//! committed. None of these records is a second prompt state: every field is read from
//! the committed prompt record and its answer receipt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::prompts::AnswerDelivery;
use crate::errors::OpId;
use crate::schema::common::Target;
use crate::schema::ids::{ProcessToken, PromptId, TerminalId};
use crate::schema::methods::status::{
    PromptAnswer, PromptKind, PromptMechanism, PromptRecord, PromptState,
};
use crate::schema::scalar::{Count, EventSeq, Incarnation, OpenEnum, RecordVersion};

/// The credential fields of a request a program authenticates with its process token.
///
/// It is decoded before the closed parameter type, ignoring every other field, so a
/// caller without a current token is refused without learning the request's shape.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct ProgramCredential {
    /// Token handed to the program at spawn.
    pub process_token: ProcessToken,
    /// Terminal the request names, when it names one.
    #[serde(default)]
    pub target: Option<Target>,
    /// Prompt the request names, when it names one.
    #[serde(default)]
    pub prompt_id: Option<PromptId>,
}

/// One accepted answer, delivered to the extension of the program that opened the prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExtensionEvent {
    /// Prompt.
    pub prompt_id: PromptId,
    /// Version of the committed answered record; the acknowledgement must name it.
    pub version: RecordVersion,
    /// Program incarnation that opened the prompt.
    pub process_incarnation: Incarnation,
    /// Kind of the prompt.
    pub kind: OpenEnum<PromptKind>,
    /// The answer, validated against the prompt's choices, fields or text bound.
    pub answer: PromptAnswer,
    /// Operation whose receipt committed the answer.
    pub answer_op_id: OpId,
    /// Sequence of the closing event; extension events are delivered in this order.
    pub answered_seq: EventSeq,
}

impl ExtensionEvent {
    /// The event of a prompt answered through its extension mechanism; any other record,
    /// open, dismissed, typed or answered by RPC event, has none.
    #[must_use]
    pub fn from_answered(record: &PromptRecord) -> Option<Self> {
        let receipt = record.answer.as_ref()?;
        let answered = record.state == OpenEnum::Known(PromptState::Answered)
            && record.mechanism == OpenEnum::Known(PromptMechanism::Extension)
            && receipt.delivery == OpenEnum::Known(AnswerDelivery::Extension);
        answered.then(|| Self {
            prompt_id: record.prompt_id.clone(),
            version: record.version,
            process_incarnation: record.process_incarnation,
            kind: record.kind.clone(),
            answer: receipt.answer.clone(),
            answer_op_id: receipt.op_id.clone(),
            answered_seq: receipt.answered_seq,
        })
    }
}

/// Where the delivery of one extension answer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeliveryState {
    /// Committed and not yet handed to an extension session.
    Pending,
    /// Handed to the current session and awaiting its acknowledgement.
    Delivered,
    /// Acknowledged at the committed version.
    Acknowledged,
    /// Handed to a session that ended before acknowledging; it is never handed out again.
    OutcomeUnknown,
}

/// The delivery of one answer and the receipt it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DeliveryRecord {
    /// Prompt.
    pub prompt_id: PromptId,
    /// Version of the committed answered record.
    pub version: RecordVersion,
    /// Operation whose receipt committed the answer.
    pub answer_op_id: OpId,
    /// Delivery state.
    pub state: OpenEnum<DeliveryState>,
}

impl DeliveryRecord {
    /// The delivery of `event` in `state`.
    #[must_use]
    pub fn new(event: &ExtensionEvent, state: DeliveryState) -> Self {
        Self {
            prompt_id: event.prompt_id.clone(),
            version: event.version,
            answer_op_id: event.answer_op_id.clone(),
            state: OpenEnum::Known(state),
        }
    }
}

/// A bridge session opened for one program incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BridgeAttachment {
    /// Terminal running the program.
    pub terminal_id: TerminalId,
    /// Program incarnation.
    pub process_incarnation: Incarnation,
    /// Most delivered, unacknowledged events the session holds at once.
    pub queue_limit: Count,
}

impl BridgeAttachment {
    /// A session of program `process_incarnation` on `terminal_id` holding `queue_limit`.
    #[must_use]
    pub fn new(
        (terminal_id, process_incarnation): (TerminalId, Incarnation),
        queue_limit: Count,
    ) -> Self {
        Self {
            terminal_id,
            process_incarnation,
            queue_limit,
        }
    }
}

#[cfg(test)]
mod tests;
