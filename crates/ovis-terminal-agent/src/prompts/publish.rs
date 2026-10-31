//! `prompt.open` and `prompt.close` by the program that owns the prompt.

use ovis_terminal_core::events::ResourceId;
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_protocol::domain::prompts::PromptOwner;
use ovis_terminal_protocol::schema::ids::{PromptId, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::status::{
    PromptCloseOutcome, PromptRecord, PromptState,
};
use ovis_terminal_store::Snapshot;

use super::request::validate;
use super::service::settle;
use super::store::{self, OPEN_LIMIT, Row};
use super::{Close, Open, PromptError, PromptService};
use crate::status::{AgentStatusEvent, StatusService};

impl PromptService {
    /// Opens a prompt as the program holding the token, or republishes the open prompt it
    /// names at the next version; the record binds the program incarnation and its agent.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `invalid_prompt`, `limit_exceeded`) settle the operation as
    /// failed; a store or decoding failure is returned.
    pub fn open<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        open: Open,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self.status.tokens().authorize(&open.token, open.terminal)?;
            validate(&open)?;
            let seq = StatusService::unit_event(snapshot);
            let affected = vec![ResourceId::Terminal(owner.terminal)];
            let existing = match &open.prompt_id {
                Some(id) => store::prompt(snapshot, self.runtime, id)?,
                None => None,
            };
            if let Some(mut stored) = existing {
                let record = &mut stored.value.record;
                if stored.value.terminal != owner.terminal
                    || record.process_incarnation != owner.process
                {
                    return Err(PromptError::NotOwner);
                }
                if !record.is_open() {
                    return Err(PromptError::InvalidPrompt {
                        field: "params.prompt_id",
                    });
                }
                record.republish(open.definition);
                let event = AgentStatusEvent::PromptUpdated(Box::new(record.clone()));
                return Ok(Completion::new(serde_json::to_value(&*record)?)
                    .write(stored.write()?)
                    .event(D::from(event), affected));
            }
            let mut index = store::index(snapshot, self.runtime)?;
            if index.value.open_on(owner.terminal).count() >= OPEN_LIMIT {
                return Err(PromptError::LimitExceeded);
            }
            let id = open
                .prompt_id
                .unwrap_or_else(|| PromptId::new(format!("prompt:{}", uuid::Uuid::now_v7())));
            let wire = WireTerminal::new(owner.terminal.to_string());
            let place = (id.clone(), PromptOwner::new(wire, owner.process));
            let mut record = PromptRecord::opened(place, open.definition, seq);
            let status = self
                .status
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?
                .row
                .status;
            record.agent_id = status.agent_id;
            record.session = status.session;
            index.value.add(id.clone(), owner.terminal);
            let row = Row {
                terminal: owner.terminal,
                record: record.clone(),
            };
            let row = store::created(snapshot, self.runtime, row)?;
            self.add_route(id, owner.terminal)?;
            Ok(Completion::new(serde_json::to_value(&record)?)
                .write(row.write()?)
                .write(index.write()?)
                .event(
                    D::from(AgentStatusEvent::PromptOpened(Box::new(record))),
                    affected,
                ))
        })())
    }

    /// Closes an open prompt as the program that opened it, at the next version.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `not_found`, `already_closed`) settle the operation as
    /// failed; a store or decoding failure is returned.
    pub fn close<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        close: &Close,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self
                .status
                .tokens()
                .authorize(&close.token, close.terminal)?;
            let mut stored = store::prompt(snapshot, self.runtime, &close.prompt_id)?
                .filter(|stored| stored.value.terminal == close.terminal)
                .ok_or(PromptError::NotFound)?;
            let record = &mut stored.value.record;
            if record.process_incarnation != owner.process {
                return Err(PromptError::NotOwner);
            }
            if !record.is_open() {
                return Err(PromptError::AlreadyClosed);
            }
            let state = match close.outcome {
                PromptCloseOutcome::Answered => PromptState::Answered,
                PromptCloseOutcome::Dismissed => PromptState::Dismissed,
                _ => PromptState::Expired,
            };
            record.close(state, StatusService::unit_event(snapshot));
            let record = record.clone();
            let place = (self.runtime, close.terminal);
            let mut completion = Completion::new(serde_json::to_value(&record)?)
                .write(stored.write()?)
                .event(
                    D::from(AgentStatusEvent::PromptClosed(Box::new(record))),
                    vec![ResourceId::Terminal(close.terminal)],
                );
            for write in store::close_in_index(snapshot, place, &close.prompt_id)? {
                completion = completion.write(write);
            }
            Ok(completion)
        })())
    }
}
