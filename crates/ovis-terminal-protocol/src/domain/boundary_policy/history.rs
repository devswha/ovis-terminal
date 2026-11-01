//! Construction of `agent.history` listings over authorized provider stores.

use crate::schema::common::PageInfo;
use crate::schema::methods::agent::AgentHistoryResult;
use crate::schema::records::SessionRef;

impl SessionRef {
    /// A session found in provider store `source`, identified by `kind` and `value`.
    #[must_use]
    pub fn new(source: String, (kind, value): (String, String)) -> Self {
        Self {
            source,
            kind,
            value,
        }
    }
}

impl PageInfo {
    /// Marks the listing as cut short by a size bound.
    #[must_use]
    pub const fn cut_short(mut self) -> Self {
        self.truncated = true;
        self
    }
}

impl AgentHistoryResult {
    /// One page of sessions with its paging facts.
    #[must_use]
    pub const fn new(sessions: Vec<SessionRef>, page: PageInfo) -> Self {
        Self { sessions, page }
    }
}
