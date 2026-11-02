//! `terminal.*` reads served from VT state; none of them writes to the program.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::graphics::{GraphicsIndexEntry, GraphicsPlaceholder};
use crate::schema::common::{OutputCursor, Pattern, Target, TextFormat};
use crate::schema::method::{MethodSpec, method};
use crate::schema::records::{Modes, ScreenCursor, StatusSnapshot};
use crate::schema::scalar::{ByteCount, ByteOffset, DurationMs, LineNumber, OpenEnum, Revision};

/// Parameters of `terminal.screen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalScreenParams {
    /// Terminal to read.
    pub target: Target,
    /// Text shape.
    pub format: TextFormat,
    /// Also return `image_index`, the fuller placement index: every stored image with
    /// its placement on either screen, never its payload. Absent means false; a present
    /// value must be a JSON boolean.
    #[serde(default)]
    pub include_images: bool,
}

/// Result of `terminal.screen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalScreenResult {
    /// Screen revision.
    pub revision: Revision,
    /// Raw output offset the screen reflects.
    pub output_offset: ByteOffset,
    /// Visible lines.
    pub lines: Vec<String>,
    /// Cursor state.
    pub cursor: ScreenCursor,
    /// Terminal modes.
    pub modes: Modes,
    /// Window title.
    pub title: Option<String>,
    /// Whether the reply was cut short by the frame bound.
    pub truncated: bool,
    /// One placeholder per stored image shown on the screen, top row first, carried by
    /// every read (empty when none); the cells each covers read as blank text in `lines`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GraphicsPlaceholder>>,
    /// Every stored image in receipt order with its placement; present only when
    /// `include_images` was requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_index: Option<Vec<GraphicsIndexEntry>>,
}

/// Parameters of `terminal.history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalHistoryParams {
    /// Terminal to read.
    pub target: Target,
    /// First line to return; absent starts at the newest lines.
    pub from_line: Option<LineNumber>,
    /// Maximum number of lines.
    pub limit: u32,
    /// Text shape.
    pub format: TextFormat,
}

/// Why history is not available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryGap {
    /// The program uses the alternate screen, which keeps no history.
    AltScreen,
    /// The requested lines were evicted.
    Evicted,
    /// Only metadata survived a reboot.
    RestoredMetadata,
}

/// Result of `terminal.history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalHistoryResult {
    /// Returned lines.
    pub lines: Vec<String>,
    /// Number of the first returned line.
    pub first_line: LineNumber,
    /// Oldest retained line.
    pub floor_line: LineNumber,
    /// Screen revision the lines belong to.
    pub revision: Revision,
    /// Why some or all requested lines are missing.
    pub gap: Option<OpenEnum<HistoryGap>>,
    /// Whether more lines follow.
    pub truncated: bool,
}

/// Parameters of `terminal.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalSearchParams {
    /// Terminal to search.
    pub target: Target,
    /// Pattern to find.
    pub pattern: Pattern,
    /// Revision the search must run against.
    pub revision: Option<Revision>,
    /// Maximum number of matches.
    pub limit: u32,
}

/// One search match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SearchMatch {
    /// History line of the match.
    pub line: LineNumber,
    /// Column of the first matched cell.
    pub col: u16,
    /// Matched length in cells.
    pub len: u16,
}

/// Result of `terminal.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalSearchResult {
    /// Matches in line order.
    pub matches: Vec<SearchMatch>,
    /// Revision searched.
    pub revision: Revision,
    /// Whether every retained line was searched.
    pub complete: bool,
    /// Oldest line that could be searched.
    pub floor_line: LineNumber,
}

/// Parameters of `terminal.output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalOutputParams {
    /// Terminal to read.
    pub target: Target,
    /// Raw output offset to start at.
    pub from_offset: ByteOffset,
    /// Maximum bytes to return.
    pub limit_bytes: ByteCount,
}

/// Result of `terminal.output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalOutputResult {
    /// Raw bytes, base64-encoded.
    pub data_b64: String,
    /// Offset of the first returned byte.
    pub from_offset: ByteOffset,
    /// Offset after the last returned byte.
    pub next_offset: ByteOffset,
    /// Oldest retained offset.
    pub floor_offset: ByteOffset,
    /// Whether bytes between the request and the floor were evicted.
    pub gap: bool,
}

/// Parameters of `terminal.wait_output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminalWaitOutputParams {
    /// Terminal to watch.
    pub target: Target,
    /// Cursor captured before or with the send; output after it counts.
    pub cursor: OutputCursor,
    /// Pattern to wait for.
    pub pattern: Pattern,
    /// Wait bound.
    pub timeout_ms: DurationMs,
}

/// How an output wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WaitOutputOutcome {
    /// The pattern matched.
    Matched,
    /// The bound passed.
    Timeout,
    /// The program exited first.
    Exited,
    /// The terminal was lost first.
    Lost,
    /// The cursor fell behind retention.
    Gap,
}

/// Result of `terminal.wait_output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TerminalWaitOutputResult {
    /// How the wait ended.
    pub outcome: OpenEnum<WaitOutputOutcome>,
    /// Offset of the match, when matched.
    pub matched_at: Option<ByteOffset>,
    /// Cursor to continue from.
    pub cursor: OutputCursor,
    /// Declared status when the wait ended.
    pub status: StatusSnapshot,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"terminal.screen", Query, Terminal, TerminalScreenParams => TerminalScreenResult,
    "Read the visible screen without touching the program.", [FormatUnsupported, FrameTooLarge]},
    method! {"terminal.history", Query, Terminal, TerminalHistoryParams => TerminalHistoryResult,
    "Read retained history lines with explicit gaps.", [CursorExpired, HistoryUnavailable]},
    method! {"terminal.search", Query, Terminal, TerminalSearchParams => TerminalSearchResult,
    "Search retained history at one revision.", [RevisionMismatch, InvalidPattern, HistoryUnavailable]},
    method! {"terminal.output", Query, Terminal, TerminalOutputParams => TerminalOutputResult,
    "Read retained raw output by offset.", [OffsetEvicted, FrameTooLarge]},
    method! {"terminal.wait_output", Query, Terminal, TerminalWaitOutputParams => TerminalWaitOutputResult,
    "Wait for output after a cursor, including output printed during a send.", [CursorExpired, InvalidPattern]},
];
