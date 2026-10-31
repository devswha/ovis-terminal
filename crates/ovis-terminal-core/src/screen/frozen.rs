//! An immutable copy of the screen and scrollback at one revision; every
//! line read is answered from one of these, never from the live engine.

use ovis_terminal_vt::{
    ContentRevision, HistoryRead, LineSpan, ScreenBuffer, ScreenLine, ScreenRead, Vt,
};

use super::error::ScreenError;
use super::views::{HistoryState, SearchHit, UnavailableReason};

fn narrow(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

#[derive(Debug)]
pub(super) struct Frozen {
    pub(super) screen: ScreenRead,
    pub(super) history: HistoryState,
    /// Scrollback lines first, then the visible rows.
    lines: Vec<ScreenLine>,
    history_len: u32,
}

impl Frozen {
    pub(super) fn capture(vt: &Vt, ended: bool) -> Self {
        let screen = vt.screen();
        // The engine's only answer besides lines is an unavailable history; a
        // later variant is treated the same, never as an empty success.
        let (history, mut lines) = if let HistoryRead::Lines {
            completeness,
            lines,
            ..
        } = vt.history(LineSpan::new(0, u32::MAX))
        {
            (HistoryState::Available { completeness }, lines)
        } else {
            let reason = if ended && screen.buffer == ScreenBuffer::Alternate {
                UnavailableReason::EndedInAlternateScreen
            } else {
                UnavailableReason::AlternateScreen
            };
            let unavailability = reason.into();
            (HistoryState::Unavailable { unavailability }, Vec::new())
        };
        let history_len = narrow(lines.len());
        lines.extend(screen.lines.iter().cloned());
        Self {
            screen,
            history,
            lines,
            history_len,
        }
    }

    pub(super) const fn revision(&self) -> ContentRevision {
        self.screen.revision
    }

    pub(super) const fn history_len(&self) -> u32 {
        self.history_len
    }

    pub(super) fn all_lines(&self) -> LineSpan {
        LineSpan::new(0, narrow(self.lines.len()))
    }

    fn span(&self, span: LineSpan, bound: u32) -> Option<&[ScreenLine]> {
        let end = span.first.checked_add(span.count)?;
        if end > bound {
            return None;
        }
        self.lines
            .get(usize::try_from(span.first).ok()?..usize::try_from(end).ok()?)
    }

    /// Scrollback lines `span`, clipped to the scrollback length.
    pub(super) fn page(&self, span: LineSpan) -> Result<&[ScreenLine], ScreenError> {
        let available = self.history_len.saturating_sub(span.first);
        let clipped = LineSpan::new(span.first, span.count.min(available));
        self.span(clipped, self.history_len)
            .ok_or(ScreenError::InvalidRange {
                field: "cursor.line",
            })
    }

    pub(super) fn search(&self, needle: &str, limit: u32) -> (Vec<SearchHit>, bool) {
        let mut hits = Vec::new();
        let cap = usize::try_from(limit).unwrap_or(usize::MAX);
        for (index, line) in self.lines.iter().enumerate() {
            for (byte, _) in line.text.match_indices(needle) {
                if hits.len() == cap {
                    return (hits, false);
                }
                let char_index = line.text.get(..byte).map_or(0, |head| head.chars().count());
                hits.push(SearchHit {
                    line: narrow(index),
                    char_index: narrow(char_index),
                });
            }
        }
        (hits, true)
    }

    pub(super) fn selection(&self, span: LineSpan) -> Result<String, ScreenError> {
        let lines = self
            .span(span, narrow(self.lines.len()))
            .ok_or(ScreenError::InvalidRange { field: "lines" })?;
        let mut text = String::new();
        for line in lines {
            text.push_str(&line.text);
            if !line.wrapped {
                text.push('\n');
            }
        }
        Ok(text)
    }
}
