//! Scrollback reads plus revision-guarded search and selection (IDEAL-036,
//! IDEAL-047). Line indexes count from the oldest retained history line; the
//! visible rows follow the history.

use std::ops::Range;

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::Line;
use alacritty_terminal::term::cell::Cell;
use serde::{Deserialize, Serialize};

use crate::engine::Vt;
use crate::engine::error::VtError;
use crate::engine::fidelity::Fidelity;
use crate::engine::modes::ScreenBuffer;
use crate::engine::read::{ScreenLine, row_text};
use crate::engine::units::{ContentRevision, OutputOffset};

/// A contiguous run of history lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LineSpan {
    /// Index of the first line, counting from the oldest retained line.
    pub first: u32,
    /// Number of lines.
    pub count: u32,
}

impl LineSpan {
    /// Builds a span.
    pub const fn new(first: u32, count: u32) -> Self {
        Self { first, count }
    }
}

/// Whether older lines may have been evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
#[non_exhaustive]
pub enum HistoryCompleteness {
    /// Every line since the last clear is retained.
    Complete,
    /// History reached its limit, so older lines may have been evicted.
    LimitReached {
        /// The configured limit.
        limit: u32,
    },
}

/// Why history cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryUnavailableReason {
    /// A full-screen program owns the alternate screen, which has no history.
    AlternateScreen,
}

/// Result of a history read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
#[non_exhaustive]
pub enum HistoryRead {
    /// Lines were returned.
    Lines {
        /// Output offset the read reflects.
        offset: OutputOffset,
        /// Content revision the read reflects.
        revision: ContentRevision,
        /// Span the caller asked for.
        requested: LineSpan,
        /// Span actually returned, clipped to retained history.
        returned: LineSpan,
        /// Retained history lines.
        total_lines: u32,
        /// `returned` is shorter than `requested`.
        truncated: bool,
        /// Eviction state.
        completeness: HistoryCompleteness,
        /// Exactness of the underlying state.
        fidelity: Fidelity,
        /// The lines, oldest first.
        lines: Vec<ScreenLine>,
    },
    /// No history exists for the active screen.
    Unavailable {
        /// Why.
        reason: HistoryUnavailableReason,
        /// Output offset the answer reflects.
        offset: OutputOffset,
        /// Content revision the answer reflects.
        revision: ContentRevision,
    },
}

/// One search hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchMatch {
    /// Line index: history lines first, then visible rows.
    pub line: u32,
    /// Character index of the match within the line text.
    pub char_index: u32,
}

fn line_at(grid: &Grid<Cell>, index: usize) -> Option<ScreenLine> {
    let history = i32::try_from(grid.history_size()).ok()?;
    let line = i32::try_from(index).ok()?.checked_sub(history)?;
    (index < grid.total_lines()).then(|| row_text(grid, Line(line)))
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl Vt {
    /// Reads primary-screen history lines without scrolling the viewport.
    pub fn history(&self, requested: LineSpan) -> HistoryRead {
        let grid = self.term.grid();
        if self.modes.screen.buffer == ScreenBuffer::Alternate {
            return HistoryRead::Unavailable {
                reason: HistoryUnavailableReason::AlternateScreen,
                offset: self.offset,
                revision: self.revision,
            };
        }
        let total = saturating_u32(grid.history_size());
        let first = requested.first.min(total);
        let end = requested.first.saturating_add(requested.count).min(total);
        let lines: Vec<ScreenLine> = (first..end)
            .filter_map(|index| usize::try_from(index).ok())
            .filter_map(|index| line_at(grid, index))
            .collect();
        let limit = self.config.history_limit.get();
        HistoryRead::Lines {
            offset: self.offset,
            revision: self.revision,
            requested,
            returned: LineSpan::new(first, saturating_u32(lines.len())),
            total_lines: total,
            truncated: end.saturating_sub(first) < requested.count,
            completeness: if total >= limit {
                HistoryCompleteness::LimitReached { limit }
            } else {
                HistoryCompleteness::Complete
            },
            fidelity: self.fidelity.clone(),
            lines,
        }
    }

    fn guard(&self, expected: ContentRevision) -> Result<&Grid<Cell>, VtError> {
        if expected == self.revision {
            Ok(self.term.grid())
        } else {
            Err(VtError::revision_mismatch(expected, self.revision))
        }
    }

    /// Finds `needle` in history and visible rows of the active screen.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::RevisionMismatch`] when `expected` is not the
    /// current content revision.
    pub fn search(
        &self,
        needle: &str,
        expected: ContentRevision,
    ) -> Result<Vec<SearchMatch>, VtError> {
        let grid = self.guard(expected)?;
        let mut matches = Vec::new();
        for index in 0..grid.total_lines() {
            let Some(line) = line_at(grid, index) else {
                continue;
            };
            for (byte, _) in line.text.match_indices(needle) {
                let char_index = line
                    .text
                    .get(..byte)
                    .map_or(0, |prefix| prefix.chars().count());
                matches.push(SearchMatch {
                    line: saturating_u32(index),
                    char_index: saturating_u32(char_index),
                });
            }
        }
        Ok(matches)
    }

    /// Returns the text of a line range, joining soft-wrapped lines.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::RevisionMismatch`] when `expected` is not the
    /// current content revision.
    pub fn selection_text(
        &self,
        lines: Range<u32>,
        expected: ContentRevision,
    ) -> Result<String, VtError> {
        let grid = self.guard(expected)?;
        let mut text = String::new();
        for index in lines.filter_map(|index| usize::try_from(index).ok()) {
            let Some(line) = line_at(grid, index) else {
                break;
            };
            text.push_str(&line.text);
            if !line.wrapped {
                text.push('\n');
            }
        }
        Ok(text)
    }
}
