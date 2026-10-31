//! `prompt.answer`: liveness, version, state, schema and input admission in one unit.
//!
//! The coordinator builds one unit at a time, so of two answers naming the same version
//! exactly one finds the prompt open; the other sees the advanced version.

use ovis_terminal_core::events::ResourceId;
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_protocol::schema::methods::status::{
    AnswerDelivery, PromptAnswer, PromptAnswerReceipt, PromptState,
};
use ovis_terminal_store::Snapshot;

use super::request::accepted;
use super::service::settle;
use super::{Answer, AnswerKeys, PromptError, PromptService, store};
use crate::status::{AgentStatusEvent, ProcessOwner, StatusService};

impl PromptService {
    /// Answers the named version of an open prompt of the terminal's current program.
    ///
    /// The receipt, the closed record and its `prompt.closed` event commit together. A
    /// prompt whose program is no longer current is expired in the same unit instead.
    ///
    /// # Errors
    ///
    /// Refusals (`not_found`, `version_conflict`, `prompt_closed`, `answer_invalid`,
    /// `delivery_unsupported` and input refusals) settle the operation as failed with no
    /// byte admitted; a store or decoding failure is returned.
    pub fn answer<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        answer: Answer,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let mut stored = store::prompt(snapshot, self.runtime, &answer.prompt_id)?
                .filter(|stored| stored.value.terminal == answer.terminal)
                .ok_or(PromptError::NotFound)?;
            let seq = StatusService::unit_event(snapshot);
            let record = &mut stored.value.record;
            let current = self.status.tokens().current(answer.terminal)?;
            let live = current.is_some_and(|owner| owner.process == record.process_incarnation);
            let affected = vec![ResourceId::Terminal(answer.terminal)];
            let place = (self.runtime, answer.terminal);
            let mut completion = if !live && record.is_open() {
                record.close(PromptState::Expired, seq);
                Completion::failed(PromptError::PromptClosed.error_data())
            } else {
                if answer.version != record.version {
                    return Err(PromptError::VersionConflict {
                        current: record.version,
                    });
                }
                if !record.is_open() {
                    return Err(PromptError::PromptClosed);
                }
                let delivery = accepted(record, &answer.answer)?;
                if let (AnswerDelivery::Typed, PromptAnswer::Text(text)) =
                    (delivery, &answer.answer)
                {
                    let keys = self.keys.as_ref().ok_or(PromptError::DeliveryUnsupported)?;
                    let owner = ProcessOwner::new(answer.terminal, record.process_incarnation);
                    let keys_request = AnswerKeys {
                        owner,
                        lease: answer.lease.clone(),
                        text: text.clone(),
                    };
                    keys.admit(&keys_request).map_err(PromptError::Input)?;
                }
                record.close(PromptState::Answered, seq);
                let given = (answer.op_id, answer.answer);
                record.answer = Some(PromptAnswerReceipt::new(given, delivery, seq));
                Completion::new(serde_json::to_value(&*record)?)
            };
            let event = AgentStatusEvent::PromptClosed(Box::new(record.clone()));
            completion = completion
                .write(stored.write()?)
                .event(D::from(event), affected);
            for write in store::close_in_index(snapshot, place, &answer.prompt_id)? {
                completion = completion.write(write);
            }
            Ok(completion)
        })())
    }
}
