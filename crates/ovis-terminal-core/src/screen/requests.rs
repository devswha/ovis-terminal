//! Read requests in their closed schema form; counters are decimal strings.

use std::num::NonZeroU32;

use ovis_terminal_vt::{ContentRevision, LineSpan, OutputOffset};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::error::ScreenError;

/// Parses a request at the boundary; numeric counters and unknown fields are refused.
///
/// # Errors
///
/// Returns [`ScreenError::InvalidParams`] for any schema violation.
pub fn decode_request<T: DeserializeOwned>(json: &[u8]) -> Result<T, ScreenError> {
    serde_json::from_slice(json).map_err(ScreenError::InvalidParams)
}

/// A position in one pinned snapshot's scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PageCursor {
    /// Snapshot revision.
    pub revision: ContentRevision,
    /// Line index in that snapshot.
    pub line: u32,
}

impl PageCursor {
    /// Builds a cursor.
    pub const fn new(revision: ContentRevision, line: u32) -> Self {
        Self { revision, line }
    }
}

/// Parameters of a history read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<PageCursor>,
    limit: NonZeroU32,
}

impl HistoryRequest {
    /// First page, from the oldest retained line, at the live revision.
    pub const fn new(limit: NonZeroU32) -> Self {
        Self {
            cursor: None,
            limit,
        }
    }

    /// Continues at a cursor of a pinned snapshot.
    #[must_use]
    pub const fn at(self, cursor: PageCursor) -> Self {
        Self {
            cursor: Some(cursor),
            ..self
        }
    }

    pub(super) const fn cursor(&self) -> Option<PageCursor> {
        self.cursor
    }

    pub(super) const fn limit(&self) -> u32 {
        self.limit.get()
    }
}

/// Literal text to find; never empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Needle(String);

impl Needle {
    /// Wraps search text.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenError::InvalidRange`] for empty text.
    pub fn new(text: String) -> Result<Self, ScreenError> {
        if text.is_empty() {
            return Err(ScreenError::InvalidRange { field: "needle" });
        }
        Ok(Self(text))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Needle {
    type Error = ScreenError;

    fn try_from(text: String) -> Result<Self, ScreenError> {
        Self::new(text)
    }
}

impl From<Needle> for String {
    fn from(needle: Needle) -> Self {
        needle.0
    }
}

/// Parameters of a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<ContentRevision>,
    needle: Needle,
    limit: NonZeroU32,
}

impl SearchRequest {
    /// Searches the live revision.
    pub const fn new(needle: Needle, limit: NonZeroU32) -> Self {
        Self {
            revision: None,
            needle,
            limit,
        }
    }

    /// Searches a pinned revision instead.
    #[must_use]
    pub fn at(self, revision: ContentRevision) -> Self {
        Self {
            revision: Some(revision),
            ..self
        }
    }

    pub(super) const fn revision(&self) -> Option<ContentRevision> {
        self.revision
    }

    pub(super) const fn needle(&self) -> &Needle {
        &self.needle
    }

    pub(super) const fn limit(&self) -> u32 {
        self.limit.get()
    }
}

/// Parameters of a revision-guarded selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    revision: ContentRevision,
    lines: LineSpan,
}

impl SelectionRequest {
    /// Selects `lines` of the snapshot at `revision`.
    pub const fn new(revision: ContentRevision, lines: LineSpan) -> Self {
        Self { revision, lines }
    }

    pub(super) const fn revision(&self) -> ContentRevision {
        self.revision
    }

    pub(super) const fn lines(&self) -> LineSpan {
        self.lines
    }
}

/// Parameters of a raw output read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRequest {
    from: OutputOffset,
    limit_bytes: NonZeroU32,
}

impl OutputRequest {
    /// Reads up to `limit_bytes` starting at `from`.
    pub const fn new(from: OutputOffset, limit_bytes: NonZeroU32) -> Self {
        Self { from, limit_bytes }
    }

    pub(super) const fn from(&self) -> OutputOffset {
        self.from
    }

    pub(super) const fn limit_bytes(&self) -> u32 {
        self.limit_bytes.get()
    }
}
