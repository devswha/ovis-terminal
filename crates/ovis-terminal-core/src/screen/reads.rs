//! Terminal reads: screen, history, search and selection are answered from a
//! pinned immutable snapshot; none of them writes to the program, scrolls,
//! or touches viewers.

use std::sync::Arc;

use ovis_terminal_vt::{ContentRevision, LineSpan};

use super::error::ScreenError;
use super::frozen::Frozen;
use super::pipeline::Screen;
use super::requests::{HistoryRequest, OutputRequest, PageCursor, SearchRequest, SelectionRequest};
use super::views::{
    HistoryPage, HistoryState, LiveState, OutputSlice, ReadMeta, RetainedRange, ScreenView,
    SearchResult, SelectionText,
};

struct Pinned {
    frozen: Arc<Frozen>,
    live: LiveState,
    retained: RetainedRange,
}

impl Pinned {
    fn meta(&self) -> ReadMeta {
        let screen = &self.frozen.screen;
        ReadMeta {
            source: screen.source,
            format: screen.format,
            revision: screen.revision,
            offset: screen.offset,
            live: LiveState {
                at_live_bottom: screen.revision == self.live.revision,
                ..self.live
            },
            history: self.frozen.history,
            retained: self.retained,
            fidelity: screen.fidelity.clone(),
        }
    }
}

impl Screen {
    /// Returns the snapshot at `revision`, or pins the live one when absent.
    fn pinned(&self, revision: Option<ContentRevision>) -> Result<Pinned, ScreenError> {
        let mut inner = self.lock()?;
        let live = inner.live();
        let wanted = revision.unwrap_or(live.revision);
        let frozen = match inner.pins.find(wanted) {
            Some(frozen) => frozen,
            None if wanted == live.revision => {
                let frozen = Arc::new(Frozen::capture(&inner.vt, inner.ended.is_some()));
                inner.pins.insert(Arc::clone(&frozen));
                frozen
            }
            None => {
                return Err(ScreenError::StaleRevision {
                    requested: wanted,
                    current: live.revision,
                });
            }
        };
        Ok(Pinned {
            frozen,
            live,
            retained: inner.ring.range(),
        })
    }

    /// Reads the visible screen at the live revision.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::Poisoned`] when the state lock is poisoned.
    pub fn screen(&self) -> Result<ScreenView, ScreenError> {
        let pinned = self.pinned(None)?;
        let screen = &pinned.frozen.screen;
        Ok(ScreenView {
            meta: pinned.meta(),
            mode_revision: screen.mode_revision,
            geometry: screen.geometry,
            buffer: screen.buffer,
            lines: screen.lines.clone(),
            cursor: screen.cursor,
            modes: screen.modes,
            title: screen.title.clone(),
        })
    }

    /// Reads one scrollback page; later pages stay on the first page's revision.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::StaleRevision`] when the cursor's snapshot is
    /// gone, [`ScreenError::HistoryUnavailable`] with its recovery when the
    /// snapshot has no scrollback, and [`ScreenError::InvalidRange`] for a
    /// cursor past the scrollback.
    pub fn history(&self, request: &HistoryRequest) -> Result<HistoryPage, ScreenError> {
        let cursor = request.cursor();
        let pinned = self.pinned(cursor.map(|cursor| cursor.revision))?;
        let frozen = &pinned.frozen;
        if let HistoryState::Unavailable { unavailability } = frozen.history {
            return Err(ScreenError::HistoryUnavailable {
                unavailability,
                revision: frozen.revision(),
                offset: frozen.screen.offset,
            });
        }
        let requested = LineSpan::new(cursor.map_or(0, |cursor| cursor.line), request.limit());
        let lines = frozen.page(requested)?.to_vec();
        let returned = LineSpan::new(
            requested.first,
            u32::try_from(lines.len()).unwrap_or(u32::MAX),
        );
        let end = returned.first.saturating_add(returned.count);
        let next = (end < frozen.history_len()).then(|| PageCursor::new(frozen.revision(), end));
        Ok(HistoryPage {
            meta: pinned.meta(),
            requested,
            returned,
            total_lines: frozen.history_len(),
            truncated: next.is_some(),
            next,
            lines,
        })
    }

    /// Finds literal text in scrollback and visible rows of one snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::StaleRevision`] when the named snapshot is gone.
    pub fn search(&self, request: &SearchRequest) -> Result<SearchResult, ScreenError> {
        let pinned = self.pinned(request.revision())?;
        let (matches, complete) = pinned
            .frozen
            .search(request.needle().as_str(), request.limit());
        Ok(SearchResult {
            meta: pinned.meta(),
            searched: pinned.frozen.all_lines(),
            matches,
            complete,
        })
    }

    /// Returns the text of a line range of the snapshot at the named revision.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::StaleRevision`] when the snapshot is gone and
    /// [`ScreenError::InvalidRange`] for lines outside it.
    pub fn selection(&self, request: &SelectionRequest) -> Result<SelectionText, ScreenError> {
        let pinned = self.pinned(Some(request.revision()))?;
        let text = pinned.frozen.selection(request.lines())?;
        Ok(SelectionText {
            meta: pinned.meta(),
            lines: request.lines(),
            text,
        })
    }

    /// Reads retained raw output by byte offset.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::HistoryGap`] with the retained range and the
    /// covering checkpoint for an evicted start, and
    /// [`ScreenError::InvalidRange`] for a start past the head.
    pub fn output(&self, request: &OutputRequest) -> Result<OutputSlice, ScreenError> {
        let inner = self.lock()?;
        let mark = inner.checkpoint.as_ref().map(|(mark, _)| mark);
        inner.ring.read(request, mark)
    }
}
