//! Typed construction of versioned prompt records and their answers (plan todo 34).
//!
//! A prompt is a record of its own: its version advances on every republish, answer and
//! close, and nothing about a turn ending changes it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::ids::{PromptId, TerminalId};
use crate::schema::methods::status::{
    PromptAnswer, PromptChoice, PromptDefinition, PromptField, PromptKind, PromptListResult,
    PromptMechanism, PromptRecord, PromptState,
};
use crate::schema::scalar::{EventSeq, Incarnation, OpenEnum, RecordVersion};

/// How an accepted answer reached the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AnswerDelivery {
    /// Admitted as typed input through the terminal's input path.
    Typed,
    /// Published as the prompt's closing event.
    Event,
    /// Published for the program's in-process extension.
    Extension,
}

/// The accepted answer of a prompt and the operation that committed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PromptAnswerReceipt {
    /// Operation that answered; a retry with it returns this receipt.
    pub op_id: OpId,
    /// The answer, kept as data.
    pub answer: PromptAnswer,
    /// How it was delivered.
    pub delivery: OpenEnum<AnswerDelivery>,
    /// Sequence of the closing event.
    pub answered_seq: EventSeq,
}

impl PromptChoice {
    /// A choice `id` shown as `label`.
    #[must_use]
    pub const fn new(id: String, label: String) -> Self {
        Self { id, label }
    }
}

impl PromptField {
    /// A form field named `name`.
    #[must_use]
    pub const fn new(name: String, required: bool) -> Self {
        Self { name, required }
    }
}

impl PromptDefinition {
    /// A `kind` prompt asking `text`, answered through `mechanism`, with no choices or fields.
    #[must_use]
    pub const fn new(kind: PromptKind, text: String, mechanism: PromptMechanism) -> Self {
        Self {
            kind,
            text,
            choices: Vec::new(),
            fields: Vec::new(),
            mechanism,
        }
    }
}

/// Where an opened prompt lives: its terminal and the program incarnation that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PromptOwner {
    /// Terminal.
    pub terminal_id: TerminalId,
    /// Program incarnation.
    pub process_incarnation: Incarnation,
}

impl PromptOwner {
    /// Program `process_incarnation` of `terminal_id`.
    #[must_use]
    pub const fn new(terminal_id: TerminalId, process_incarnation: Incarnation) -> Self {
        Self {
            terminal_id,
            process_incarnation,
        }
    }
}

impl PromptRecord {
    /// Version 1 of `definition`, open, published by `owner` at event `opened_seq`.
    #[must_use]
    pub fn opened(
        (prompt_id, owner): (PromptId, PromptOwner),
        definition: PromptDefinition,
        opened_seq: EventSeq,
    ) -> Self {
        Self {
            prompt_id,
            version: RecordVersion::new(1),
            terminal_id: owner.terminal_id,
            process_incarnation: owner.process_incarnation,
            agent_id: None,
            session: None,
            kind: OpenEnum::Known(definition.kind),
            text: definition.text,
            choices: definition.choices,
            fields: definition.fields,
            mechanism: OpenEnum::Known(definition.mechanism),
            state: OpenEnum::Known(PromptState::Open),
            opened_seq,
            closed_seq: None,
            answer: None,
        }
    }

    /// Whether the prompt still awaits an answer.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.state, OpenEnum::Known(PromptState::Open))
    }

    /// Replaces the question with `definition` at the next version.
    pub fn republish(&mut self, definition: PromptDefinition) {
        self.kind = OpenEnum::Known(definition.kind);
        self.text = definition.text;
        self.choices = definition.choices;
        self.fields = definition.fields;
        self.mechanism = OpenEnum::Known(definition.mechanism);
        self.bump();
    }

    /// Closes the prompt in `state` at event `seq`, at the next version.
    pub fn close(&mut self, state: PromptState, seq: EventSeq) {
        self.state = OpenEnum::Known(state);
        self.closed_seq = Some(seq);
        self.bump();
    }

    const fn bump(&mut self) {
        self.version = RecordVersion::new(self.version.get().saturating_add(1));
    }
}

impl PromptAnswerReceipt {
    /// `answer`, committed by `op_id` and delivered by `delivery` at event `answered_seq`.
    #[must_use]
    pub fn new(
        (op_id, answer): (OpId, PromptAnswer),
        delivery: AnswerDelivery,
        answered_seq: EventSeq,
    ) -> Self {
        Self {
            op_id,
            answer,
            delivery: OpenEnum::Known(delivery),
            answered_seq,
        }
    }
}

impl PromptListResult {
    /// The listed `prompts`, oldest first.
    #[must_use]
    pub const fn new(prompts: Vec<PromptRecord>) -> Self {
        Self { prompts }
    }
}

#[cfg(test)]
mod tests;
