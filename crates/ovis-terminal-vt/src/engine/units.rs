//! Offsets, revisions and geometry: the coordinates every read and checkpoint carries.

use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::engine::error::VtError;

/// Cumulative count of terminal output bytes the engine has consumed.
///
/// Offsets saturate at `u64::MAX`, a byte count no terminal reaches.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OutputOffset(#[serde(with = "crate::engine::decimal")] u64);

impl std::fmt::Display for OutputOffset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl OutputOffset {
    /// The offset before any output byte.
    pub const ZERO: Self = Self(0);

    /// Wraps a byte count.
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The byte count.
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(super) fn advanced_by(self, len: usize) -> Self {
        Self(
            self.0
                .saturating_add(u64::try_from(len).unwrap_or(u64::MAX)),
        )
    }
}

/// Monotonic counter of completed control functions and printed characters.
///
/// It depends only on the byte stream, never on how the stream was chunked, so
/// a restored engine and an always-live engine agree on it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ContentRevision(#[serde(with = "crate::engine::decimal")] u64);

impl std::fmt::Display for ContentRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ContentRevision {
    /// The raw counter value.
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(super) const fn advanced_by(self, steps: u64) -> Self {
        Self(self.0.saturating_add(steps))
    }
}

/// Monotonic counter bumped whenever the structured terminal modes change.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ModeRevision(#[serde(with = "crate::engine::decimal")] u64);

impl std::fmt::Display for ModeRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ModeRevision {
    /// The raw counter value.
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(super) const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Narrowest grid the emulator handles: alacritty 0.26 panics when a wide
/// character is printed into a single column.
pub const MIN_COLUMNS: u16 = 2;

#[derive(Serialize, Deserialize)]
struct RawGeometry {
    columns: u16,
    rows: u16,
}

/// Grid size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawGeometry", into = "RawGeometry")]
pub struct Geometry {
    columns: NonZeroU16,
    rows: NonZeroU16,
}

impl TryFrom<RawGeometry> for Geometry {
    type Error = VtError;

    fn try_from(raw: RawGeometry) -> Result<Self, VtError> {
        Self::new(raw.columns, raw.rows)
    }
}

impl From<Geometry> for RawGeometry {
    fn from(geometry: Geometry) -> Self {
        Self {
            columns: geometry.columns(),
            rows: geometry.rows(),
        }
    }
}

impl Geometry {
    /// Builds a geometry from cell counts.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::InvalidGeometry`] for fewer than [`MIN_COLUMNS`]
    /// columns or zero rows.
    pub const fn new(columns: u16, rows: u16) -> Result<Self, VtError> {
        match (NonZeroU16::new(columns), NonZeroU16::new(rows)) {
            (Some(wide), Some(tall)) if columns >= MIN_COLUMNS => Ok(Self {
                columns: wide,
                rows: tall,
            }),
            _ => Err(VtError::InvalidGeometry { columns, rows }),
        }
    }

    /// Number of columns.
    pub const fn columns(self) -> u16 {
        self.columns.get()
    }

    /// Number of rows.
    pub const fn rows(self) -> u16 {
        self.rows.get()
    }

    pub(super) fn column_count(self) -> usize {
        usize::from(self.columns.get())
    }

    pub(super) fn row_count(self) -> usize {
        usize::from(self.rows.get())
    }
}

/// Maximum number of scrollback lines the primary screen retains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HistoryLimit(u32);

impl HistoryLimit {
    /// Wraps a line count.
    pub const fn new(lines: u32) -> Self {
        Self(lines)
    }

    /// The line count.
    pub const fn get(self) -> u32 {
        self.0
    }

    pub(super) fn line_count(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// Engine construction parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VtConfig {
    /// Initial grid size.
    pub geometry: Geometry,
    /// Primary-screen scrollback bound.
    pub history_limit: HistoryLimit,
}

impl VtConfig {
    /// Builds a configuration.
    pub const fn new(geometry: Geometry, history_limit: HistoryLimit) -> Self {
        Self {
            geometry,
            history_limit,
        }
    }
}
