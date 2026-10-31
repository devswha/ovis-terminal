//! Session binding and versioned conversation cursors (plan todo 36).
//!
//! A cursor names one history identity at one observed revision; a page is served only
//! when both still match, so records of two revisions never mix in one listing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::common::{PageInfo, PageToken};
use crate::schema::ids::{AgentId, HistoryId, RecordId};
use crate::schema::methods::agent::{AgentConversationResult, ConversationRecord};
use crate::schema::records::SessionRef;
use crate::schema::scalar::{HistoryRevision, Incarnation, OpenEnum, RecordPosition, Revision};

/// Why an owning process changed the session it is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionChange {
    /// The process resumed an existing provider session.
    Resume,
    /// The process selected another chain of the same history (fork or rewind).
    Branch,
    /// The process replaced its session with a different one.
    Replace,
}

/// What an observation found different from the recorded history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryChange {
    /// Records were added after the recorded content, which is unchanged.
    Append,
    /// The source is shorter than the recorded content.
    Truncate,
    /// The source is a different file than the one recorded.
    Replace,
    /// Recorded bytes changed in place, including rewrites that keep the size.
    Rewrite,
    /// The active chain was selected explicitly by the owning process.
    Branch,
    /// The binding moved to another history identity.
    Rebind,
}

/// A page position bound to one history identity, revision and chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ConversationCursor {
    /// History the position belongs to.
    pub history_id: HistoryId,
    /// Revision the position was issued at.
    pub revision: HistoryRevision,
    /// Records of the chain already returned.
    pub position: RecordPosition,
    /// Last record of the chain being read; absent for the active chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<RecordId>,
}

impl ConversationCursor {
    /// Position `position` of the active chain of `history_id` at `revision`.
    #[must_use]
    pub const fn new(
        history_id: HistoryId,
        revision: HistoryRevision,
        position: RecordPosition,
    ) -> Self {
        Self {
            history_id,
            revision,
            position,
            head: None,
        }
    }

    /// The opaque `page.next` form of this cursor; [`Self::from_token`] reverses it.
    ///
    /// # Errors
    ///
    /// Returns an encoding failure.
    pub fn token(&self) -> Result<PageToken, serde_json::Error> {
        serde_json::to_string(self).map(PageToken::new)
    }

    /// Reads a cursor back from its `page.next` form.
    ///
    /// # Errors
    ///
    /// Returns a decoding failure for a token this runtime did not issue.
    pub fn from_token(token: &PageToken) -> Result<Self, serde_json::Error> {
        serde_json::from_str(token.as_str())
    }
}

/// Identity and revision a history has now, and why it moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct HistoryHead {
    /// Current history identity.
    pub history_id: HistoryId,
    /// Current revision.
    pub revision: HistoryRevision,
    /// What differed from the presented cursor, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<OpenEnum<HistoryChange>>,
}

impl HistoryHead {
    /// `history_id` at `revision`, moved by `change`.
    #[must_use]
    pub const fn new(
        history_id: HistoryId,
        revision: HistoryRevision,
        change: Option<HistoryChange>,
    ) -> Self {
        let change = match change {
            Some(change) => Some(OpenEnum::Known(change)),
            None => None,
        };
        Self {
            history_id,
            revision,
            change,
        }
    }
}

/// Payload of `agent.session_bound`: one explicit binding change of an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SessionBindingRecord {
    /// Agent whose binding changed.
    pub agent_id: AgentId,
    /// Program incarnation that reported the change.
    pub process_incarnation: Incarnation,
    /// Session now bound.
    pub session: SessionRef,
    /// History now bound, with its revision.
    pub history: HistoryHead,
    /// Active chain head selected by the owning process, when one was selected.
    pub head: Option<RecordId>,
    /// Declared reason; absent for the first binding of the incarnation.
    pub reason: Option<OpenEnum<SessionChange>>,
    /// Session bound before this change.
    pub previous_session: Option<SessionRef>,
    /// History bound before this change.
    pub previous_history: Option<HistoryId>,
    /// Binding revision after this change.
    pub binding_revision: Revision,
}

/// Every field of a [`SessionBindingRecord`] except its wire wrappers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "the agent crate builds binding records from every field; a new field must fail its build"
)]
pub struct BindingFacts {
    /// Agent whose binding changed.
    pub agent_id: AgentId,
    /// Reporting program incarnation.
    pub process_incarnation: Incarnation,
    /// Session now bound.
    pub session: SessionRef,
    /// History now bound.
    pub history: HistoryHead,
    /// Explicitly selected chain head.
    pub head: Option<RecordId>,
    /// Declared reason.
    pub reason: Option<SessionChange>,
    /// Session bound before.
    pub previous_session: Option<SessionRef>,
    /// History bound before.
    pub previous_history: Option<HistoryId>,
    /// Binding revision after the change.
    pub binding_revision: Revision,
}

impl From<BindingFacts> for SessionBindingRecord {
    fn from(facts: BindingFacts) -> Self {
        Self {
            agent_id: facts.agent_id,
            process_incarnation: facts.process_incarnation,
            session: facts.session,
            history: facts.history,
            head: facts.head,
            reason: facts.reason.map(OpenEnum::Known),
            previous_session: facts.previous_session,
            previous_history: facts.previous_history,
            binding_revision: facts.binding_revision,
        }
    }
}

impl ConversationRecord {
    /// Record `record_id` under `parent`, with `role` and provider `blocks`.
    #[must_use]
    pub fn new(
        record_id: RecordId,
        parent: Option<RecordId>,
        (role, blocks): (String, Vec<serde_json::Value>),
    ) -> Self {
        Self {
            record_id,
            parent,
            role,
            blocks,
            provenance: String::new(),
        }
    }
}

/// One page of one chain of a history at one revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "the agent crate builds pages from every field; a new field must fail its build"
)]
pub struct ConversationPage {
    /// History and revision every record belongs to.
    pub history_id: HistoryId,
    /// Revision of the observation the page was cut from.
    pub revision: HistoryRevision,
    /// Last record of the chain read; absent for an empty history.
    pub head: Option<RecordId>,
    /// Whether the chain is the active one.
    pub active: bool,
    /// Leaf records outside the chain read, each addressable through a cursor head.
    pub off_branch: Vec<RecordId>,
    /// Records of the chain from the requested position.
    pub records: Vec<ConversationRecord>,
    /// Records in the whole chain.
    pub total: u32,
    /// Continuation, absent on the last page.
    pub next: Option<ConversationCursor>,
}

impl TryFrom<ConversationPage> for AgentConversationResult {
    type Error = serde_json::Error;

    fn try_from(page: ConversationPage) -> Result<Self, Self::Error> {
        let token = page
            .next
            .as_ref()
            .map(ConversationCursor::token)
            .transpose()?;
        Ok(Self {
            history_id: page.history_id,
            revision: page.revision,
            head: page.head,
            active: page.active,
            off_branch: page.off_branch,
            records: page.records,
            page: PageInfo::new(Revision::new(page.revision.get()), page.total, token),
            next_cursor: page.next,
        })
    }
}

impl PageInfo {
    /// Paging facts of a listing pinned to `revision` with `total` entries.
    #[must_use]
    pub const fn new(revision: Revision, total: u32, next: Option<PageToken>) -> Self {
        Self {
            revision,
            total,
            next,
            truncated: false,
        }
    }
}

#[cfg(test)]
mod tests;
