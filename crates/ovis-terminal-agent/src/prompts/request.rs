//! Prompt requests and their validation against bounded definitions and answer schemas.

use std::collections::BTreeSet;

use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{LeaseId, ProcessToken, PromptId};
use ovis_terminal_protocol::schema::methods::status::{
    AnswerDelivery, PromptAnswer, PromptCloseOutcome, PromptDefinition, PromptMechanism,
    PromptRecord,
};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, RecordVersion};

use super::PromptError;

/// Largest identifier, choice id, label or field name, in bytes.
const NAME_LIMIT: usize = 256;
/// Largest question, answer text or field value, in bytes.
const TEXT_LIMIT: usize = 4096;
/// Most choices or fields one prompt declares.
const ITEM_LIMIT: usize = 16;

/// A `prompt.open` by the process holding `token`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Open {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the program.
    pub token: ProcessToken,
    /// The program's identity for the prompt; an open one it owns is republished.
    pub prompt_id: Option<PromptId>,
    /// The question.
    pub definition: PromptDefinition,
}

impl Open {
    /// Opens `definition` on `terminal` for the program holding `token`.
    #[must_use]
    pub const fn new(
        terminal: TerminalId,
        token: ProcessToken,
        definition: PromptDefinition,
    ) -> Self {
        Self {
            terminal,
            token,
            prompt_id: None,
            definition,
        }
    }
}

/// A `prompt.answer` by an authorized principal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Answer {
    /// Terminal the prompt was routed to.
    pub terminal: TerminalId,
    /// Operation committing the answer.
    pub op_id: OpId,
    /// The prompt.
    pub prompt_id: PromptId,
    /// The version the caller answers.
    pub version: RecordVersion,
    /// The answer.
    pub answer: PromptAnswer,
    /// Interact lease, for typed answers.
    pub lease: Option<LeaseId>,
}

impl Answer {
    /// Answers version `version` of `prompt_id` on `terminal` under `op_id`.
    #[must_use]
    pub fn new(
        (terminal, op_id): (TerminalId, OpId),
        (prompt_id, version): (PromptId, RecordVersion),
        answer: PromptAnswer,
    ) -> Self {
        Self {
            terminal,
            op_id,
            prompt_id,
            version,
            answer,
            lease: None,
        }
    }
}

/// A `prompt.close` by the program that opened the prompt.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Close {
    /// Terminal the prompt was routed to.
    pub terminal: TerminalId,
    /// Token presented by the program.
    pub token: ProcessToken,
    /// The prompt.
    pub prompt_id: PromptId,
    /// How it closed.
    pub outcome: PromptCloseOutcome,
}

impl Close {
    /// Closes `prompt_id` on `terminal` with `outcome` for the program holding `token`.
    #[must_use]
    pub fn new(
        (terminal, token): (TerminalId, ProcessToken),
        prompt_id: PromptId,
        outcome: PromptCloseOutcome,
    ) -> Self {
        Self {
            terminal,
            token,
            prompt_id,
            outcome,
        }
    }
}

fn names_ok<'a>(names: impl ExactSizeIterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    names.len() <= ITEM_LIMIT
        && names
            .into_iter()
            .all(|name| !name.is_empty() && name.len() <= NAME_LIMIT && seen.insert(name))
}

/// Accepts a bounded definition: text, at most one of choices or fields, and only text
/// for prompts typed through the terminal.
pub(super) fn validate(open: &Open) -> Result<(), PromptError> {
    let invalid = |field| Err(PromptError::InvalidPrompt { field });
    let definition = &open.definition;
    if open
        .prompt_id
        .as_ref()
        .is_some_and(|id| id.as_str().is_empty() || id.as_str().len() > NAME_LIMIT)
    {
        return invalid("params.prompt_id");
    }
    if definition.text.is_empty() || definition.text.len() > TEXT_LIMIT {
        return invalid("params.prompt.text");
    }
    let choices = &definition.choices;
    if !names_ok(choices.iter().map(|choice| choice.id.as_str()))
        || choices.iter().any(|choice| choice.label.len() > NAME_LIMIT)
    {
        return invalid("params.prompt.choices");
    }
    let fields = &definition.fields;
    if !names_ok(fields.iter().map(|field| field.name.as_str()))
        || (!fields.is_empty()
            && (!choices.is_empty() || definition.mechanism == PromptMechanism::Keys))
    {
        return invalid("params.prompt.fields");
    }
    Ok(())
}

const fn text_ok(text: &str) -> bool {
    !text.is_empty() && text.len() <= TEXT_LIMIT
}

fn schema_ok(record: &PromptRecord, answer: &PromptAnswer) -> Result<(), PromptError> {
    let invalid = |field| Err(PromptError::AnswerInvalid { field });
    match answer {
        PromptAnswer::Choice(id) if !record.choices.is_empty() => {
            if record.choices.iter().any(|choice| &choice.id == id) {
                Ok(())
            } else {
                invalid("params.answer.choice")
            }
        }
        PromptAnswer::Fields(values) if !record.fields.is_empty() => {
            let declared = |name: &String| record.fields.iter().any(|field| &field.name == name);
            let complete = record
                .fields
                .iter()
                .all(|field| !field.required || values.contains_key(&field.name));
            if complete
                && values
                    .iter()
                    .all(|(name, value)| declared(name) && value.len() <= TEXT_LIMIT)
            {
                Ok(())
            } else {
                invalid("params.answer.fields")
            }
        }
        PromptAnswer::Text(text) if record.choices.is_empty() && record.fields.is_empty() => {
            if text_ok(text) {
                Ok(())
            } else {
                invalid("params.answer.text")
            }
        }
        _ => invalid("params.answer"),
    }
}

/// How `answer` reaches the program, once it matches the prompt's declared mechanism and
/// schema. A typed prompt takes literal text only, so no choice becomes a guessed key.
pub(super) fn accepted(
    record: &PromptRecord,
    answer: &PromptAnswer,
) -> Result<AnswerDelivery, PromptError> {
    match (&record.mechanism, answer) {
        (OpenEnum::Known(PromptMechanism::Keys), PromptAnswer::Text(text)) => {
            if text_ok(text) {
                Ok(AnswerDelivery::Typed)
            } else {
                Err(PromptError::AnswerInvalid {
                    field: "params.answer.text",
                })
            }
        }
        (OpenEnum::Known(PromptMechanism::Rpc), _) => {
            schema_ok(record, answer).map(|()| AnswerDelivery::Event)
        }
        (OpenEnum::Known(PromptMechanism::Extension), _) => {
            schema_ok(record, answer).map(|()| AnswerDelivery::Extension)
        }
        _ => Err(PromptError::DeliveryUnsupported),
    }
}
