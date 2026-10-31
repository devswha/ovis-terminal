//! Read results: every one states source, format, revision, offset, live
//! bottom, history availability, retention floor and exactness.

use ovis_terminal_vt::{
    ContentRevision, CursorState, Fidelity, Geometry, HistoryCompleteness, LineSpan, ModeRevision,
    OscRecord, OutputOffset, ReadFormat, ReadSource, ScreenBuffer, ScreenLine, TerminalModes,
};
use serde::Serialize;

use super::requests::PageCursor;

/// The newest state, independent of any pinned snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LiveState {
    /// Live content revision.
    pub revision: ContentRevision,
    /// Live output offset.
    pub offset: OutputOffset,
    /// The read reflects the live revision, not an older pinned one.
    pub at_live_bottom: bool,
}

/// Raw output bytes still retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RetainedRange {
    /// Oldest retained byte; everything before it was evicted.
    pub floor: OutputOffset,
    /// Offset after the newest byte.
    pub head: OutputOffset,
}

/// A VT checkpoint boundary: the screen at `offset`, replayable with the bytes after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CheckpointMark {
    /// Output offset the checkpoint describes.
    pub offset: OutputOffset,
    /// Content revision at that offset.
    pub revision: ContentRevision,
    /// Whether checkpoint plus retained bytes reproduce the live screen exactly.
    pub fidelity: Fidelity,
}

/// Why history cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnavailableReason {
    /// A full-screen program owns the alternate screen, which keeps no history.
    AlternateScreen,
    /// The program ended while the alternate screen was active.
    EndedInAlternateScreen,
}

/// What ends an unavailability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Recovery {
    /// Ingestion reports [`ScreenTransition::HistoryAvailable`] when it ends.
    HistoryAvailableTransition,
    /// Nothing will make this history available.
    Permanent,
}

/// A reason paired with its recovery, so callers never poll to find out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Unavailability {
    /// Why.
    pub reason: UnavailableReason,
    /// What ends it.
    pub recovery: Recovery,
}

impl From<UnavailableReason> for Unavailability {
    fn from(reason: UnavailableReason) -> Self {
        let recovery = match reason {
            UnavailableReason::AlternateScreen => Recovery::HistoryAvailableTransition,
            UnavailableReason::EndedInAlternateScreen => Recovery::Permanent,
        };
        Self { reason, recovery }
    }
}

/// History availability at the read's revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
#[non_exhaustive]
pub enum HistoryState {
    /// Scrollback exists; completeness says whether older lines were evicted.
    Available {
        /// Eviction state.
        completeness: HistoryCompleteness,
    },
    /// No scrollback exists.
    Unavailable {
        /// Why, and what ends it.
        unavailability: Unavailability,
    },
}

/// Facts every line read carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ReadMeta {
    /// Where the data comes from.
    pub source: ReadSource,
    /// Line representation.
    pub format: ReadFormat,
    /// Revision of the snapshot that answered.
    pub revision: ContentRevision,
    /// Output offset of that snapshot.
    pub offset: OutputOffset,
    /// Live state when the read was served.
    pub live: LiveState,
    /// History availability of that snapshot.
    pub history: HistoryState,
    /// Raw output retention when the read was served.
    pub retained: RetainedRange,
    /// Exactness of the snapshot.
    pub fidelity: Fidelity,
}

/// The visible screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ScreenView {
    /// Read facts.
    pub meta: ReadMeta,
    /// Mode revision.
    pub mode_revision: ModeRevision,
    /// Grid size.
    pub geometry: Geometry,
    /// Active buffer.
    pub buffer: ScreenBuffer,
    /// Visible rows, top to bottom.
    pub lines: Vec<ScreenLine>,
    /// Cursor.
    pub cursor: CursorState,
    /// Structured modes.
    pub modes: TerminalModes,
    /// Window title.
    pub title: Option<String>,
}

/// One page of scrollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct HistoryPage {
    /// Read facts.
    pub meta: ReadMeta,
    /// Span the caller asked for.
    pub requested: LineSpan,
    /// Span returned.
    pub returned: LineSpan,
    /// Scrollback lines in the snapshot.
    pub total_lines: u32,
    /// More lines follow this page.
    pub truncated: bool,
    /// Continuation at the same pinned revision.
    pub next: Option<PageCursor>,
    /// Lines, oldest first.
    pub lines: Vec<ScreenLine>,
}

/// One search hit; lines count history first, then visible rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SearchHit {
    /// Line index.
    pub line: u32,
    /// Character index within the line text.
    pub char_index: u32,
}

/// Search result over one snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SearchResult {
    /// Read facts.
    pub meta: ReadMeta,
    /// Lines searched.
    pub searched: LineSpan,
    /// Hits in line order.
    pub matches: Vec<SearchHit>,
    /// Every hit is listed; false when the limit cut the list.
    pub complete: bool,
}

/// Text of a line range from one snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SelectionText {
    /// Read facts.
    pub meta: ReadMeta,
    /// Lines selected.
    pub lines: LineSpan,
    /// Text, soft-wrapped lines joined.
    pub text: String,
}

/// Retained raw output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct OutputSlice {
    /// First byte returned.
    pub from: OutputOffset,
    /// Offset after the last byte returned; the next cursor.
    pub next: OutputOffset,
    /// Raw bytes.
    pub bytes: Vec<u8>,
    /// Retained bytes follow `next`.
    pub truncated: bool,
    /// Retention when served.
    pub retained: RetainedRange,
}

/// A named change ingestion observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "transition")]
#[non_exhaustive]
pub enum ScreenTransition {
    /// History stopped being readable.
    HistoryUnavailable {
        /// Why, and what ends it.
        unavailability: Unavailability,
        /// Offset of the change.
        offset: OutputOffset,
        /// Revision after the change.
        revision: ContentRevision,
    },
    /// History became readable again.
    HistoryAvailable {
        /// Offset of the change.
        offset: OutputOffset,
        /// Revision after the change.
        revision: ContentRevision,
    },
    /// A checkpoint boundary was taken.
    Checkpoint(CheckpointMark),
}

/// What one holder chunk did to the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Ingested {
    /// Offset after the chunk.
    pub offset: OutputOffset,
    /// Revision after the chunk.
    pub revision: ContentRevision,
    /// Transitions in stream order.
    pub transitions: Vec<ScreenTransition>,
    /// OSC records for their owner; the screen derives nothing from them.
    pub osc: Vec<OscRecord>,
    /// Query replies added to the internal reply lane.
    pub queued_replies: u32,
}
