//! `agent.records`: pages of the reported records at one revision, with the status turns
//! they belong to, answered from one committed snapshot without an operation receipt.

use ovis_terminal_protocol::domain::conversation_records::{
    ConversationSource, Unavailable, UnavailableReason,
};
use ovis_terminal_protocol::domain::session_binding::{
    ConversationCursor, ConversationPage, HistoryChange, HistoryHead,
};
use ovis_terminal_protocol::schema::methods::agent::{AgentConversationResult, ConversationRecord};
use ovis_terminal_protocol::schema::scalar::{RecordPosition, TurnNumber};
use ovis_terminal_store::Snapshot;

use super::store::{Entry, Head, Keys};
use super::turns::TurnFacts;
use super::{RecordsError, RecordsService};
use crate::conversation::binding::Read;
use crate::status::row::Row;

/// Most record bytes one page carries beyond its first record.
const PAGE_BYTES: usize = 4 * 1024 * 1024;

/// Refuses a cursor that was not issued for `head` at its current revision.
fn current(cursor: &ConversationCursor, head: &Head) -> Result<(), RecordsError> {
    let moved = |change| RecordsError::HistoryChanged {
        current: HistoryHead::new(head.history_id.clone(), head.revision, change),
    };
    if cursor.history_id != head.history_id {
        return Err(moved(Some(HistoryChange::Rebind)));
    }
    if cursor.head.is_some() || cursor.revision > head.revision {
        return Err(RecordsError::CursorExpired);
    }
    // The records only grow, so an older revision differs from this one by appends alone.
    if cursor.revision < head.revision {
        return Err(moved(Some(HistoryChange::Append)));
    }
    Ok(())
}

/// Records from `position`, at most `limit` of them and about [`PAGE_BYTES`] of bytes.
fn window(
    (snapshot, keys, head): (&Snapshot, &Keys, &Head),
    position: u64,
    limit: usize,
) -> Result<Vec<ConversationRecord>, RecordsError> {
    let mut records = Vec::new();
    let mut bytes = 0_usize;
    for at in (position..head.count).take(limit) {
        let entry = Entry::<ConversationRecord>::read(snapshot, keys.record(at))?;
        let record = entry.value.ok_or(RecordsError::CursorExpired)?;
        bytes = bytes.saturating_add(serde_json::to_vec(&record)?.len());
        if !records.is_empty() && bytes > PAGE_BYTES {
            break;
        }
        records.push(record);
    }
    Ok(records)
}

/// One page of the records `head` describes, from the position `read` names.
fn cut(
    (snapshot, keys, head): (&Snapshot, &Keys, &Head),
    row: &Row,
    read: &Read,
) -> Result<ConversationPage, RecordsError> {
    let cursor = read.continued().map_err(|_| RecordsError::CursorExpired)?;
    if let Some(cursor) = &cursor {
        current(cursor, head)?;
    }
    let position = cursor.as_ref().map_or(0, |cursor| cursor.position.get());
    if position > head.count {
        return Err(RecordsError::CursorExpired);
    }
    let records = window((snapshot, keys, head), position, read.limit())?;
    let end =
        position.saturating_add(u64::try_from(records.len()).map_err(|_| RecordsError::TooLarge)?);
    let next = (end < head.count).then(|| {
        ConversationCursor::new(
            head.history_id.clone(),
            head.revision,
            RecordPosition::new(end),
        )
    });
    let mut numbers: Vec<TurnNumber> = records.iter().filter_map(|record| record.turn).collect();
    numbers.dedup();
    let mut turns = Vec::with_capacity(numbers.len());
    for turn in numbers {
        let facts = Entry::<TurnFacts>::read(snapshot, keys.turn(turn))?.value;
        turns.push(facts.unwrap_or_default().view(turn, row));
    }
    Ok(ConversationPage {
        history_id: head.history_id.clone(),
        revision: head.revision,
        head: head.last.clone(),
        active: true,
        off_branch: Vec::new(),
        records,
        total: u32::try_from(head.count).map_err(|_| RecordsError::TooLarge)?,
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
    /// Returns `conversation_unavailable`, `history_changed`, `cursor_expired`, or a store
    /// or decoding failure.
    pub fn read(
        &self,
        snapshot: &Snapshot,
        read: &Read,
    ) -> Result<AgentConversationResult, RecordsError> {
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
        let Some(agent) = stored.row.agent.as_ref() else {
            return Err(unavailable(UnavailableReason::NoAgent, (None, None)));
        };
        let keys = Keys::new(self.runtime, &agent.agent_id);
        let head = Entry::<Head>::read(snapshot, keys.head())?
            .value
            .ok_or_else(|| {
                unavailable(
                    UnavailableReason::NotReported,
                    (Some(agent.agent_id.clone()), agent.session.clone()),
                )
            })?;
        let page = cut((snapshot, &keys, &head), &stored.row, read)?;
        Ok(AgentConversationResult::try_from(page)?)
    }
}
