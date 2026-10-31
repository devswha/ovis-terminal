//! Owning-process binding updates and conversation reads, after target resolution.

use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::domain::session_binding::{ConversationCursor, SessionChange};
use ovis_terminal_protocol::schema::common::PageRequest;
use ovis_terminal_protocol::schema::ids::{HistoryId, ProcessToken, RecordId};
use ovis_terminal_protocol::schema::records::SessionRef;

/// An `agent.update_session` by the process holding `token`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Update {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the reporter.
    pub token: ProcessToken,
    /// Session now in use.
    pub session: SessionRef,
    /// History the session continues; minted when absent.
    pub history_id: Option<HistoryId>,
    /// Declared reason of the change.
    pub reason: Option<SessionChange>,
    /// Chain head selected with reason `branch`.
    pub head: Option<RecordId>,
}

impl Update {
    /// Binds `session` for the program holding `token` on `terminal`.
    #[must_use]
    pub const fn new(terminal: TerminalId, token: ProcessToken, session: SessionRef) -> Self {
        Self {
            terminal,
            token,
            session,
            history_id: None,
            reason: None,
            head: None,
        }
    }
}

/// An `agent.conversation` page request.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Read {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Structured continuation.
    pub cursor: Option<ConversationCursor>,
    /// Page request; `after` carries a cursor in its `page.next` form.
    pub page: Option<PageRequest>,
}

impl Read {
    /// The first page of the active chain of `terminal`'s bound history.
    #[must_use]
    pub const fn new(terminal: TerminalId) -> Self {
        Self {
            terminal,
            cursor: None,
            page: None,
        }
    }
}
