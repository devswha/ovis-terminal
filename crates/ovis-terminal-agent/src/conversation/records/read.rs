//! `agent.conversation` with source `records`: pages of the reported records at one
//! revision, with the status turns they belong to.

use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_protocol::domain::conversation_records::{
    ConversationSource, Unavailable, UnavailableReason,
};
use ovis_terminal_protocol::domain::session_binding::{
    ConversationCursor, ConversationPage, HistoryChange, HistoryHead,
};
use ovis_terminal_protocol::schema::methods::agent::AgentConversationResult;
use ovis_terminal_protocol::schema::scalar::{RecordPosition, TurnNumber};
use ovis_terminal_store::Snapshot;

use super::log::{RecordLog, StoredLog};
use super::service::settle;
use super::{RecordsError, RecordsService};
use crate::conversation::binding::Read;
use crate::status::AgentStatusEvent;
use crate::status::row::Row;

/// Refuses a cursor that was not issued for `log` at its current revision.
fn current(cursor: &ConversationCursor, log: &RecordLog) -> Result<(), RecordsError> {
    let moved = |change| RecordsError::HistoryChanged {
        current: HistoryHead::new(log.history_id.clone(), log.revision, change),
    };
    if cursor.history_id != log.history_id {
        return Err(moved(Some(HistoryChange::Rebind)));
    }
    if cursor.head.is_some() || cursor.revision > log.revision {
        return Err(RecordsError::CursorExpired);
    }
    // The log only grows, so an older revision differs from this one by appends alone.
    if cursor.revision < log.revision {
        return Err(moved(Some(HistoryChange::Append)));
    }
    Ok(())
}

/// One page of `log` from the position `read` names.
fn cut(log: &RecordLog, row: &Row, read: &Read) -> Result<ConversationPage, RecordsError> {
    let cursor = read.continued().map_err(|_| RecordsError::CursorExpired)?;
    if let Some(cursor) = &cursor {
        current(cursor, log)?;
    }
    let position = cursor.as_ref().map_or(Ok(0), |cursor| {
        usize::try_from(cursor.position.get()).map_err(|_| RecordsError::CursorExpired)
    })?;
    let window = log
        .records
        .get(position..)
        .ok_or(RecordsError::CursorExpired)?;
    let records: Vec<_> = window.iter().take(read.limit()).cloned().collect();
    let end = position.saturating_add(records.len());
    let next = (end < log.records.len())
        .then(|| u64::try_from(end).ok())
        .flatten()
        .map(|end| {
            ConversationCursor::new(
                log.history_id.clone(),
                log.revision,
                RecordPosition::new(end),
            )
        });
    let mut turns: Vec<TurnNumber> = records.iter().filter_map(|record| record.turn).collect();
    turns.dedup();
    let turns = turns
        .into_iter()
        .map(|turn| {
            let facts = log.turns.get(&turn).cloned().unwrap_or_default();
            facts.view(turn, row)
        })
        .collect();
    Ok(ConversationPage {
        history_id: log.history_id.clone(),
        revision: log.revision,
        head: log.records.last().map(|record| record.record_id.clone()),
        active: true,
        off_branch: Vec::new(),
        records,
        total: u32::try_from(log.records.len()).map_err(|_| RecordsError::TooLarge)?,
        next,
        turns,
    })
}

impl RecordsService {
    /// Reads one page of the records reported by the terminal's current agent.
    ///
    /// A plain program and an agent that never reported records are refused as
    /// `conversation_unavailable` naming what was consulted; terminal output is never
    /// offered in their place.
    ///
    /// # Errors
    ///
    /// Refusals (`conversation_unavailable`, `history_changed`, `cursor_expired`) settle
    /// the operation as failed; a store or decoding failure is returned.
    pub fn conversation<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        read: &Read,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let process = self
                .status
                .tokens()
                .current(read.terminal)?
                .map(|owner| owner.process);
            let stored = self
                .status
                .location(read.terminal)
                .read(snapshot, process)?;
            let unavailable = |reason, consulted| {
                RecordsError::Unavailable(Box::new(Unavailable::new(
                    reason,
                    ConversationSource::Records,
                    consulted,
                )))
            };
            let Some(agent) = stored.row.agent.clone() else {
                return Err(unavailable(UnavailableReason::NoAgent, (None, None)));
            };
            let log = StoredLog::read(snapshot, self.runtime, &agent.agent_id)?
                .log
                .ok_or_else(|| {
                    unavailable(
                        UnavailableReason::NotReported,
                        (Some(agent.agent_id.clone()), agent.session.clone()),
                    )
                })?;
            let page = cut(&log, &stored.row, read)?;
            let result = AgentConversationResult::try_from(page)?;
            Ok(Completion::new(serde_json::to_value(result)?))
        })())
    }
}
