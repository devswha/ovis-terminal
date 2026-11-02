//! `terminal.history` (IDEAL-035, IDEAL-036, IDEAL-047): retained scrollback lines of one
//! incarnation, read from the runtime-owned screen without writing to the program or
//! moving a viewport. Line numbers count from the oldest retained line, as in
//! `terminal.search`. Without `from_line` the read ends at the newest line. Both pages
//! of one read pin the same screen revision, so output arriving in between cannot shift
//! the lines. A screen without scrollback is `history_unavailable`.

use std::num::NonZeroU32;

use ovis_terminal_core::screen::{HistoryPage, HistoryRequest, PageCursor, ScreenError};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::TextFormat;
use ovis_terminal_protocol::schema::methods::terminal_read::{
    TerminalHistoryParams, TerminalHistoryResult,
};
use ovis_terminal_protocol::schema::scalar::LineNumber;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::{checked, refusal};
use super::input::live_terminal;

fn screen_refusal(error: &ScreenError) -> BoundaryError {
    refusal(error.kind(), error.hint())
}

/// The first line a read of `limit` lines starts at, given `total` retained lines: the
/// requested line, or the start of the newest `limit` lines. A line number past
/// `u32::MAX` cannot be retained, so it is clamped and later refused as out of range.
fn first_line(total: u32, from_line: Option<u64>, limit: NonZeroU32) -> u32 {
    from_line.map_or_else(
        || total.saturating_sub(limit.get()),
        |line| u32::try_from(line).unwrap_or(u32::MAX),
    )
}

/// The wire result of one history page.
fn history_result(page: &HistoryPage) -> Result<Value, BoundaryError> {
    let lines: Vec<&str> = page.lines.iter().map(|line| line.text.as_str()).collect();
    checked::<TerminalHistoryResult>(json!({
        "lines": lines,
        "first_line": page.returned.first.to_string(),
        "floor_line": "0",
        "revision": page.meta.revision.get().to_string(),
        "gap": null,
        "truncated": page.truncated,
    }))
}

impl Runtime {
    /// Serves one `terminal.history` read on the actor thread.
    pub(super) fn history(
        &mut self,
        params: &TerminalHistoryParams,
    ) -> Result<Value, BoundaryError> {
        if params.format != TextFormat::Text {
            return Err(refusal("format_unsupported", "request format text"));
        }
        let limit = NonZeroU32::new(params.limit)
            .ok_or_else(|| refusal("invalid_params", "request a positive limit"))?;
        let live = live_terminal((&self.directory, &mut self.live), &params.target)?;
        let screen = live.hub.screen();
        let head = screen
            .history(&HistoryRequest::new(limit))
            .map_err(|error| screen_refusal(&error))?;
        let first = first_line(
            head.total_lines,
            params.from_line.map(LineNumber::get),
            limit,
        );
        if first == 0 {
            return history_result(&head);
        }
        let cursor = PageCursor::new(head.meta.revision, first);
        let page = screen
            .history(&HistoryRequest::new(limit).at(cursor))
            .map_err(|error| screen_refusal(&error))?;
        history_result(&page)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::first_line;

    fn limit(lines: u32) -> NonZeroU32 {
        NonZeroU32::new(lines).unwrap_or(NonZeroU32::MIN)
    }

    #[test]
    fn without_from_line_the_read_ends_at_the_newest_line() {
        assert_eq!(first_line(120, None, limit(50)), 70);
        assert_eq!(first_line(120, None, limit(120)), 0);
    }

    #[test]
    fn a_short_history_starts_at_the_oldest_line() {
        assert_eq!(first_line(10, None, limit(50)), 0);
        assert_eq!(first_line(0, None, limit(1)), 0);
    }

    #[test]
    fn an_explicit_from_line_is_honoured_and_an_impossible_one_clamps() {
        assert_eq!(first_line(120, Some(5), limit(10)), 5);
        assert_eq!(first_line(120, Some(0), limit(10)), 0);
        assert_eq!(first_line(120, Some(u64::MAX), limit(10)), u32::MAX);
    }
}
