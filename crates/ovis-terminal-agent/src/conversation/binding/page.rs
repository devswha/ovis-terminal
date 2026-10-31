//! `agent.conversation` pages bound to one history identity and revision.
//!
//! Every read observes the transcript first. A detected change advances the revision
//! and is recorded even when the presented cursor is then refused as `history_changed`,
//! so a page never mixes records of two revisions.

use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_protocol::domain::session_binding::{
    ConversationCursor, ConversationPage, HistoryChange,
};
use ovis_terminal_protocol::schema::common::PageRequest;
use ovis_terminal_protocol::schema::ids::HistoryId;
use ovis_terminal_protocol::schema::methods::agent::{AgentConversationResult, ConversationRecord};
use ovis_terminal_protocol::schema::scalar::RecordPosition;
use ovis_terminal_store::{CasWrite, Snapshot};
use serde_json::Value;

use super::chain::History;
use super::observe::{self, Content};
use super::row::{HistoryRow, StoredHistory};
use super::service::settle;
use super::{BindingError, BindingService, Read, SourceRef};
use crate::status::AgentStatusEvent;

/// Records per page when the request names no limit.
const DEFAULT_LIMIT: usize = 50;
/// Most records one page carries.
const MAX_LIMIT: usize = 200;

/// The cursor a request continues from, given as `cursor` or as a `page.after` token.
fn continuation(
    cursor: Option<ConversationCursor>,
    page: Option<&PageRequest>,
) -> Result<Option<ConversationCursor>, BindingError> {
    let after = page
        .and_then(|page| page.after.as_ref())
        .map(|token| ConversationCursor::from_token(token).map_err(|_| BindingError::CursorExpired))
        .transpose()?;
    match (cursor, after) {
        (Some(cursor), Some(after)) if cursor != after => Err(BindingError::CursorExpired),
        (cursor, after) => Ok(cursor.or(after)),
    }
}

fn limit(page: Option<&PageRequest>) -> usize {
    page.and_then(|page| page.limit)
        .and_then(|limit| usize::try_from(limit).ok())
        .map_or(DEFAULT_LIMIT, |limit| limit.clamp(1, MAX_LIMIT))
}

/// Refuses a cursor issued for another identity or revision than `row` has now.
fn current(
    cursor: &ConversationCursor,
    (id, row): (&HistoryId, &HistoryRow),
) -> Result<(), BindingError> {
    if &cursor.history_id != id {
        let current = row.head(id, Some(HistoryChange::Rebind));
        return Err(BindingError::HistoryChanged { current });
    }
    if cursor.revision != row.revision {
        let follows = cursor.revision.get().checked_add(1) == Some(row.revision.get());
        let current = row.head(id, row.last_change.filter(|_| follows));
        return Err(BindingError::HistoryChanged { current });
    }
    Ok(())
}

/// One observed history ready to be paged.
struct Observed {
    id: HistoryId,
    row: HistoryRow,
    content: Content,
    provenance: String,
}

fn cut(observed: &Observed, read: &Read) -> Result<Value, BindingError> {
    let page = read.page.as_ref();
    let cursor = continuation(read.cursor.clone(), page)?;
    if let Some(cursor) = &cursor {
        current(cursor, (&observed.id, &observed.row))?;
    }
    let history = History::parse(&observed.content.bytes)?;
    let active = history.active_head(observed.row.selected.as_ref());
    let requested = cursor.as_ref().and_then(|cursor| cursor.head.as_ref());
    let head = match requested {
        Some(record) => Some(history.find(record).ok_or(BindingError::UnknownRecord)?),
        None => active,
    };
    let chain = head.map(|head| history.chain(head)).unwrap_or_default();
    let position = cursor.as_ref().map_or(Ok(0), |cursor| {
        usize::try_from(cursor.position.get()).map_err(|_| BindingError::CursorExpired)
    })?;
    let window = chain.get(position..).ok_or(BindingError::CursorExpired)?;
    let taken = window.len().min(limit(page));
    let records = window
        .iter()
        .take(taken)
        .filter_map(|index| history.records.get(*index))
        .map(|record| {
            let parent = record
                .parent
                .and_then(|parent| history.records.get(parent))
                .map(|parent| parent.id.clone());
            let mut out = ConversationRecord::new(
                record.id.clone(),
                parent,
                (record.role.clone(), record.blocks.clone()),
            );
            observed.provenance.clone_into(&mut out.provenance);
            out
        })
        .collect();
    let end = position.saturating_add(taken);
    let next = (end < chain.len())
        .then(|| u64::try_from(end).ok())
        .flatten()
        .map(|end| {
            let mut next = ConversationCursor::new(
                observed.id.clone(),
                observed.row.revision,
                RecordPosition::new(end),
            );
            next.head = requested.cloned();
            next
        });
    let result = AgentConversationResult::try_from(ConversationPage {
        history_id: observed.id.clone(),
        revision: observed.row.revision,
        head: head
            .and_then(|head| history.records.get(head))
            .map(|record| record.id.clone()),
        active: head == active,
        off_branch: history.off_branch(&chain),
        records,
        total: u32::try_from(chain.len()).map_err(|_| BindingError::TooLarge)?,
        next,
    })?;
    Ok(serde_json::to_value(result)?)
}

impl BindingService {
    fn observe(
        &self,
        snapshot: &Snapshot,
        read: &Read,
    ) -> Result<(Observed, Option<CasWrite>), BindingError> {
        let process = self
            .status
            .tokens()
            .current(read.terminal)?
            .map(|owner| owner.process);
        let stored = self
            .status
            .location(read.terminal)
            .read(snapshot, process)?;
        let agent = stored.row.agent.ok_or(BindingError::Unbound)?;
        let id = agent.history_id.ok_or(BindingError::Unbound)?;
        let history = StoredHistory::read(snapshot, self.runtime, &id)?;
        let mut row = history.row.clone().ok_or(BindingError::Unbound)?;
        let SourceRef::Transcript(path) = &row.source else {
            return Err(BindingError::NoTranscript);
        };
        let content = observe::read(path)?;
        let write = if row.observe(&content) {
            Some(history.write(&row)?)
        } else {
            None
        };
        let provenance = agent
            .session
            .map(|session| session.source)
            .unwrap_or_default();
        Ok((
            Observed {
                id,
                row,
                content,
                provenance,
            },
            write,
        ))
    }

    /// Reads one page of the bound history at its current revision.
    ///
    /// # Errors
    ///
    /// Refusals (`history_changed`, `cursor_expired`, `conversation_unavailable`,
    /// `not_found`) settle the operation as failed, still recording a detected change; a
    /// store or decoding failure is returned.
    pub fn conversation<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        read: &Read,
    ) -> Result<Completion<D>, OperationError> {
        let (observed, write) = match self.observe(snapshot, read) {
            Ok(observed) => observed,
            Err(refusal) => return settle(Err(refusal)),
        };
        let completion = settle(cut(&observed, read).map(Completion::new))?;
        Ok(match write {
            Some(write) => completion.write(write),
            None => completion,
        })
    }
}
