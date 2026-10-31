//! `agent.append_records`: typed records reported by the owning process, committed with
//! their log and the `agent.records_appended` event in one unit.

use std::sync::Arc;

use ovis_terminal_core::events::{EventError, ResourceId};
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_core::registry::{RuntimeId, TerminalId};
use ovis_terminal_protocol::domain::conversation_records::{
    AppendFacts, RecordInput, RecordsAppended, TurnMetadata,
};
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, TurnNumber};
use ovis_terminal_store::Snapshot;

use super::log::{RecordLog, StoredLog};
use super::{RecordsError, Violation, validate};
use crate::status::{AgentStatusEvent, StatusError, StatusService};

/// An `agent.append_records` by the process holding `token`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Append {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the reporter.
    pub token: ProcessToken,
    /// Records to append, in order.
    pub records: Vec<RecordInput>,
    /// Metadata of the running turn.
    pub turn: Option<TurnMetadata>,
}

impl Append {
    /// Appends `records` for the program holding `token` on `terminal`.
    #[must_use]
    pub const fn new(terminal: TerminalId, token: ProcessToken, records: Vec<RecordInput>) -> Self {
        Self {
            terminal,
            token,
            records,
            turn: None,
        }
    }
}

/// Typed conversation records of every agent of one runtime.
#[derive(Debug)]
pub struct RecordsService {
    pub(super) runtime: RuntimeId,
    pub(super) status: Arc<StatusService>,
}

/// Settles a refusal as a failed operation; persistence failures stay errors.
pub(super) fn settle<D>(
    outcome: Result<Completion<D>, RecordsError>,
) -> Result<Completion<D>, OperationError> {
    match outcome {
        Ok(completion) => Ok(completion),
        Err(RecordsError::Encode(error) | RecordsError::Status(StatusError::Encode(error))) => {
            Err(error.into())
        }
        Err(RecordsError::Event(error) | RecordsError::Status(StatusError::Event(error))) => {
            Err(error.into())
        }
        Err(RecordsError::Status(StatusError::Poisoned)) => Err(EventError::Poisoned.into()),
        Err(refusal) => Ok(Completion::failed(refusal.error_data())),
    }
}

/// Applies reported turn metadata to the running turn.
fn metadata(
    log: &mut RecordLog,
    meta: Option<TurnMetadata>,
    running: Option<TurnNumber>,
) -> Result<(), RecordsError> {
    let Some(meta) = meta else {
        return Ok(());
    };
    let turn = running
        .ok_or_else(|| RecordsError::invalid("turn".to_owned(), Violation::NoRunningTurn))?;
    let facts = log.turn(turn);
    if meta.model.is_some() {
        facts.model = meta.model;
    }
    if meta.reasoning.is_some() {
        facts.reasoning = meta.reasoning;
    }
    Ok(())
}

impl RecordsService {
    /// Records of the agents `status` registers.
    #[must_use]
    pub const fn new(runtime: RuntimeId, status: Arc<StatusService>) -> Self {
        Self { runtime, status }
    }

    /// Appends typed records, and metadata of the running turn, as the owning process.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `invalid_records`, `too_large`) settle the operation as
    /// failed with nothing appended; a store or decoding failure is returned.
    pub fn append<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        append: Append,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self
                .status
                .tokens()
                .authorize(&append.token, append.terminal)?;
            let stored = self
                .status
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            let agent = stored
                .row
                .agent
                .as_ref()
                .ok_or(RecordsError::Unregistered)?;
            if append.records.is_empty() && append.turn.is_none() {
                return Err(RecordsError::invalid(
                    "records".to_owned(),
                    Violation::Empty,
                ));
            }
            let store = StoredLog::read(snapshot, self.runtime, &agent.agent_id)?;
            let mut log = store.log.clone().unwrap_or_else(RecordLog::empty);
            let running = super::turns::running(&stored.row);
            let records = validate::append(&log, &append.records, running)?;
            let mut touched: Vec<TurnNumber> = records.iter().filter_map(|r| r.turn).collect();
            touched.extend(running.filter(|_| append.turn.is_some()));
            touched.sort_unstable();
            touched.dedup();
            metadata(&mut log, append.turn, running)?;
            for turn in &touched {
                log.turn(*turn);
            }
            for (turn, facts) in &mut log.turns {
                facts.observe(*turn, &stored.row);
            }
            log.records.extend(records.iter().cloned());
            log.revision = HistoryRevision::new(log.revision.get().saturating_add(1));
            let turns = touched
                .iter()
                .filter_map(|turn| Some(log.turns.get(turn)?.view(*turn, &stored.row)))
                .collect();
            let appended = RecordsAppended::from(AppendFacts {
                agent_id: agent.agent_id.clone(),
                process_incarnation: owner.process,
                history_id: log.history_id.clone(),
                revision: log.revision,
                records,
                turns,
            });
            Ok(Completion::new(serde_json::to_value(&appended)?)
                .write(store.write(&log)?)
                .event(
                    D::from(AgentStatusEvent::RecordsAppended(Box::new(appended))),
                    vec![ResourceId::Terminal(owner.terminal)],
                ))
        })())
    }
}
