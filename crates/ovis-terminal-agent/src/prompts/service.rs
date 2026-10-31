//! Prompt routing, reads and the blocked-input check of one runtime.
//!
//! Prompt state is a record of its own; a status report, a turn ending or a read never
//! changes it. Mutations live in `open`, `answer` and `close`, each one commit unit.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_core::registry::{RuntimeId, TerminalId};
use ovis_terminal_protocol::schema::ids::PromptId;
use ovis_terminal_protocol::schema::methods::status::{PromptRecord, PromptState};
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::Snapshot;

use super::{KeyInput, PromptError, store};
use crate::status::StatusService;

/// Versioned prompts of every terminal of one runtime.
#[derive(Debug)]
pub struct PromptService {
    pub(super) runtime: RuntimeId,
    pub(super) status: Arc<StatusService>,
    pub(super) keys: Option<Arc<dyn KeyInput>>,
    routes: Mutex<BTreeMap<PromptId, TerminalId>>,
}

/// Turns domain refusals into failed operations and keeps internal failures as errors.
pub(super) fn settle<D>(
    outcome: Result<Completion<D>, PromptError>,
) -> Result<Completion<D>, OperationError> {
    match outcome {
        Ok(completion) => Ok(completion),
        Err(PromptError::Encode(error)) => Err(error.into()),
        Err(PromptError::Event(error)) => Err(error.into()),
        Err(PromptError::Poisoned) => Err(EventError::Poisoned.into()),
        Err(refusal) => Ok(Completion::failed(refusal.error_data())),
    }
}

impl PromptService {
    /// Prompts of `runtime`, owned by the programs `status` binds; typed answers are
    /// refused as `delivery_unsupported` until an input path is attached.
    #[must_use]
    pub fn new(runtime: RuntimeId, status: Arc<StatusService>) -> Self {
        Self {
            runtime,
            status,
            keys: None,
            routes: Mutex::new(BTreeMap::new()),
        }
    }

    /// Admits typed answers to `keys`, the terminals' ordered input path.
    #[must_use]
    pub fn with_keys(mut self, keys: Arc<dyn KeyInput>) -> Self {
        self.keys = Some(keys);
        self
    }

    fn routes(&self) -> Result<MutexGuard<'_, BTreeMap<PromptId, TerminalId>>, PromptError> {
        self.routes.lock().map_err(|_| PromptError::Poisoned)
    }

    /// The terminal `prompt` was opened on, for authorizing requests that name only it.
    ///
    /// The stored row stays authoritative: a request routed here is refused as
    /// `not_found` unless the committed prompt is on the same terminal.
    ///
    /// # Errors
    ///
    /// Returns `not_found` for a prompt this runtime never opened.
    pub fn route(&self, prompt: &PromptId) -> Result<TerminalId, PromptError> {
        self.routes()?
            .get(prompt)
            .copied()
            .ok_or(PromptError::NotFound)
    }

    pub(super) fn add_route(
        &self,
        prompt: PromptId,
        terminal: TerminalId,
    ) -> Result<(), PromptError> {
        self.routes()?.insert(prompt, terminal);
        Ok(())
    }

    /// Reloads every retained prompt's route after a restart.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn recover(&self, snapshot: &Snapshot) -> Result<(), PromptError> {
        let index = store::index(snapshot, self.runtime)?.value;
        let mut routes = self.routes()?;
        for entry in index.entries {
            routes.insert(entry.prompt_id, entry.terminal);
        }
        drop(routes);
        Ok(())
    }

    /// Reads one prompt.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, or a store or decoding failure.
    pub fn get(&self, snapshot: &Snapshot, prompt: &PromptId) -> Result<PromptRecord, PromptError> {
        store::prompt(snapshot, self.runtime, prompt)?
            .map(|row| row.value.record)
            .ok_or(PromptError::NotFound)
    }

    /// Prompts of `terminal`, or of every terminal, in `states` when given, oldest first.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn list(
        &self,
        snapshot: &Snapshot,
        (terminal, states): (Option<TerminalId>, Option<&[PromptState]>),
    ) -> Result<Vec<PromptRecord>, PromptError> {
        let index = store::index(snapshot, self.runtime)?.value;
        let mut prompts = Vec::new();
        for entry in index.entries {
            if terminal.is_some_and(|terminal| terminal != entry.terminal) {
                continue;
            }
            let Some(row) = store::prompt(snapshot, self.runtime, &entry.prompt_id)? else {
                continue;
            };
            let record = row.value.record;
            let wanted = states.is_none_or(|states| {
                states
                    .iter()
                    .any(|state| record.state == OpenEnum::Known(*state))
            });
            if wanted {
                prompts.push(record);
            }
        }
        Ok(prompts)
    }

    /// Refuses ordinary input to `terminal` while its current program waits on an open
    /// prompt or reports `blocked`; the only ways through are `prompt.answer` and raw input
    /// under its own grant.
    ///
    /// # Errors
    ///
    /// Returns `blocked` naming the open prompts, or a store or decoding failure.
    pub fn admission(&self, snapshot: &Snapshot, terminal: TerminalId) -> Result<(), PromptError> {
        let Some(current) = self.status.tokens().current(terminal)? else {
            return Ok(());
        };
        let index = store::index(snapshot, self.runtime)?.value;
        let mut prompt_ids = Vec::new();
        for id in index.open_on(terminal) {
            if let Some(row) = store::prompt(snapshot, self.runtime, id)?
                && row.value.record.process_incarnation == current.process
            {
                prompt_ids.push(id.clone());
            }
        }
        let status = self
            .status
            .location(terminal)
            .read(snapshot, Some(current.process))?
            .row
            .status;
        let reported_blocked = status
            .root
            .is_some_and(|root| root.report.state == OpenEnum::Known(AgentState::Blocked));
        if prompt_ids.is_empty() && !reported_blocked {
            Ok(())
        } else {
            Err(PromptError::Blocked { prompt_ids })
        }
    }
}
