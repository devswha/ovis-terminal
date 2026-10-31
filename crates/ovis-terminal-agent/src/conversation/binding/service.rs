//! `agent.update_session`: an explicit, owner-only change of the session an agent is
//! bound to, committed with its history record, agent row and event in one unit.

use std::sync::Arc;

use ovis_terminal_core::events::{EventError, ResourceId};
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_protocol::domain::session_binding::{
    BindingFacts, HistoryChange, SessionBindingRecord, SessionChange,
};
use ovis_terminal_protocol::schema::ids::HistoryId;
use ovis_terminal_protocol::schema::methods::agent::AgentResult;
use ovis_terminal_protocol::schema::records::AgentBinding;
use ovis_terminal_protocol::schema::scalar::Revision;
use ovis_terminal_store::Snapshot;

use super::chain::History;
use super::row::{HistoryRow, Selection, StoredHistory};
use super::{BindingError, SourceRef, Stores, Update, observe};
use crate::status::{AgentStatusEvent, StatusError, StatusService};

/// Session bindings and history revisions of every terminal of one runtime.
#[derive(Debug)]
pub struct BindingService {
    pub(super) runtime: RuntimeId,
    pub(super) status: Arc<StatusService>,
    pub(super) stores: Stores,
}

/// Settles a refusal as a failed operation; persistence failures stay errors.
pub(super) fn settle<D>(
    outcome: Result<Completion<D>, BindingError>,
) -> Result<Completion<D>, OperationError> {
    match outcome {
        Ok(completion) => Ok(completion),
        Err(BindingError::Encode(error) | BindingError::Status(StatusError::Encode(error))) => {
            Err(error.into())
        }
        Err(BindingError::Event(error) | BindingError::Status(StatusError::Event(error))) => {
            Err(error.into())
        }
        Err(BindingError::Status(StatusError::Poisoned)) => Err(EventError::Poisoned.into()),
        Err(refusal) => Ok(Completion::failed(refusal.error_data())),
    }
}

/// Checks the declared reason against the current binding.
fn validate(agent: &AgentBinding, update: &Update) -> Result<(), BindingError> {
    let invalid = |field| Err(BindingError::InvalidSession { field });
    match (update.reason, &update.head) {
        (None, _) if agent.history_id.is_some() => invalid("reason"),
        (Some(SessionChange::Branch), None) => invalid("head"),
        (Some(SessionChange::Branch), Some(_))
            if agent.session.as_ref() != Some(&update.session)
                || update
                    .history_id
                    .as_ref()
                    .is_some_and(|id| agent.history_id.as_ref() != Some(id)) =>
        {
            invalid("history_id")
        }
        (reason, Some(_)) if reason != Some(SessionChange::Branch) => invalid("head"),
        _ => Ok(()),
    }
}

impl BindingService {
    /// Bindings over the agents `status` registers, reading transcripts inside `stores`.
    #[must_use]
    pub const fn new(runtime: RuntimeId, status: Arc<StatusService>, stores: Stores) -> Self {
        Self {
            runtime,
            status,
            stores,
        }
    }

    /// The history row refreshed against `source`; a new history starts at revision 1.
    /// An existing history keeps the source it was first bound to.
    pub(super) fn refreshed(
        stored: &StoredHistory,
        source: SourceRef,
    ) -> Result<(HistoryRow, Option<History>), BindingError> {
        let mut row = match &stored.row {
            Some(row) if row.source != source => {
                return Err(BindingError::InvalidSession {
                    field: "history_id",
                });
            }
            Some(row) => row.clone(),
            None => HistoryRow::first(source, None),
        };
        let history = match &row.source {
            SourceRef::Transcript(path) => {
                let content = observe::read(path)?;
                row.observe(&content);
                Some(History::parse(&content.bytes)?)
            }
            _ => None,
        };
        Ok((row, history))
    }

    /// Binds, resumes, branches or replaces the session of the owning program's agent.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `invalid_session`) settle the operation as failed; a store
    /// or decoding failure is returned.
    pub fn update_session<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        update: Update,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self
                .status
                .tokens()
                .authorize(&update.token, update.terminal)?;
            let mut stored = self
                .status
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            let mut agent = stored.row.agent.clone().ok_or(BindingError::Unregistered)?;
            validate(&agent, &update)?;
            let source = self.stores.source(&update.session)?;
            let id = match (&update.history_id, update.reason) {
                (Some(id), _) => id.clone(),
                (None, Some(SessionChange::Branch)) => {
                    agent
                        .history_id
                        .clone()
                        .ok_or(BindingError::InvalidSession {
                            field: "history_id",
                        })?
                }
                (None, _) => HistoryId::new(format!("history:{}", uuid::Uuid::now_v7())),
            };
            let history_store = StoredHistory::read(snapshot, self.runtime, &id)?;
            let (mut row, history) = Self::refreshed(&history_store, source)?;
            if let Some(head) = &update.head {
                let records = history
                    .as_ref()
                    .filter(|history| history.find(head).is_some())
                    .map(|history| history.records.len())
                    .ok_or(BindingError::InvalidSession { field: "head" })?;
                row.selected = Some(Selection {
                    head: head.clone(),
                    seen: u64::try_from(records).map_err(|_| BindingError::TooLarge)?,
                });
                row.advance(HistoryChange::Branch);
            }
            let revision = Revision::new(StatusService::unit_event(snapshot).get());
            let facts = BindingFacts {
                agent_id: agent.agent_id.clone(),
                process_incarnation: owner.process,
                session: update.session.clone(),
                history: row.head(&id, row.last_change),
                head: row
                    .selected
                    .as_ref()
                    .map(|selection| selection.head.clone()),
                reason: update.reason,
                previous_session: agent.session.replace(update.session.clone()),
                previous_history: agent.history_id.replace(id),
                binding_revision: revision,
            };
            agent.revision = revision;
            stored.row.status.session = Some(update.session);
            stored.row.agent = Some(agent.clone());
            let result = AgentResult::new(Some(agent), revision);
            Ok(Completion::new(serde_json::to_value(result)?)
                .write(history_store.write(&row)?)
                .write(stored.write()?)
                .event(
                    D::from(AgentStatusEvent::SessionBound(Box::new(
                        SessionBindingRecord::from(facts),
                    ))),
                    vec![ResourceId::Terminal(owner.terminal)],
                ))
        })())
    }
}
