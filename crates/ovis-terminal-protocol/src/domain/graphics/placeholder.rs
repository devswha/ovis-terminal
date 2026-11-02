//! Stored images as screen reads report them (plan todo 85): a placeholder per image
//! shown on the read screen, and the fuller placement index a read can ask for. Neither
//! ever carries a payload; the bytes stay in the terminal's image store.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::GraphicsProtocol;
use crate::schema::scalar::{ByteCount, ByteOffset, OpenEnum};

/// Identity of one stored graphics sequence.
///
/// It is the raw output offset where the sequence (or its first Kitty chunk) began: the key of the sequence's entry in the terminal's image store
/// and the offset `graphics_replayed` names, so a read and a reattach agree on it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct GraphicsImageId(ByteOffset);

impl GraphicsImageId {
    /// The image whose sequence began at raw output offset `offset`.
    #[must_use]
    pub const fn new(offset: u64) -> Self {
        Self(ByteOffset::new(offset))
    }

    /// The raw output offset the sequence began at.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// One stored image shown on a read screen, in place of the cells it covers.
///
/// Plan todo 85. It never carries the payload: the bytes stay in the image
/// store, and `bytes` is the size of the store entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsPlaceholder {
    /// The image store entry of the sequence that displays it.
    pub image_id: GraphicsImageId,
    /// Protocol of that sequence.
    pub format: OpenEnum<GraphicsProtocol>,
    /// Zero-based screen row of the first covered cell still visible.
    pub row: u16,
    /// Zero-based screen column of the first covered cell.
    pub col: u16,
    /// Covered rows still visible, from `row` down.
    pub rows: u16,
    /// Covered columns, from `col` right, clipped to the grid.
    pub cols: u16,
    /// Raw bytes the image store holds for the sequence.
    pub bytes: ByteCount,
}

/// Where one image is shown, as a read reports it; `rows` and `cols` are already
/// clipped to the visible grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlaceholderCells {
    /// First covered row.
    pub row: u16,
    /// First covered column.
    pub col: u16,
    /// Covered rows.
    pub rows: u16,
    /// Covered columns.
    pub cols: u16,
}

impl PlaceholderCells {
    /// `rows` by `cols` cells from (`row`, `col`).
    #[must_use]
    pub const fn new(at: (u16, u16), extent: (u16, u16)) -> Self {
        Self {
            row: at.0,
            col: at.1,
            rows: extent.0,
            cols: extent.1,
        }
    }
}

impl GraphicsPlaceholder {
    /// The `protocol` image stored as `image_id` with `bytes` raw bytes, shown on `cells`.
    #[must_use]
    pub const fn new(
        (image_id, protocol): (GraphicsImageId, GraphicsProtocol),
        cells: PlaceholderCells,
        bytes: ByteCount,
    ) -> Self {
        Self {
            image_id,
            format: OpenEnum::Known(protocol),
            row: cells.row,
            col: cells.col,
            rows: cells.rows,
            cols: cells.cols,
            bytes,
        }
    }
}

/// Screen a stored image is placed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GraphicsBuffer {
    /// The primary screen.
    Primary,
    /// The alternate screen.
    Alternate,
}

/// Where an indexed image was placed, by the extent its header declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsIndexPlacement {
    /// Screen it is placed on; it is shown only while that screen is active.
    pub buffer: OpenEnum<GraphicsBuffer>,
    /// Zero-based row of its first covered row still visible.
    pub row: u16,
    /// Zero-based column of its first covered cell.
    pub col: u16,
    /// Rows its header declares; one when it declares none.
    pub rows: u16,
    /// Columns its header declares; one when it declares none.
    pub cols: u16,
}

impl GraphicsIndexPlacement {
    /// A placement on `buffer` at `cells` (declared extent).
    #[must_use]
    pub const fn new(buffer: GraphicsBuffer, cells: PlaceholderCells) -> Self {
        Self {
            buffer: OpenEnum::Known(buffer),
            row: cells.row,
            col: cells.col,
            rows: cells.rows,
            cols: cells.cols,
        }
    }
}

/// One entry of a read's placement index: an image the store holds, wherever it is placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsIndexEntry {
    /// The image store entry.
    pub image_id: GraphicsImageId,
    /// Protocol of its sequence.
    pub format: OpenEnum<GraphicsProtocol>,
    /// Raw bytes the image store holds for it.
    pub bytes: ByteCount,
    /// Where it is placed; null for a Kitty transmission that displays nothing itself.
    pub placement: Option<GraphicsIndexPlacement>,
}

impl GraphicsIndexEntry {
    /// The `protocol` image stored as `image_id` with `bytes` raw bytes, placed at `placement`.
    #[must_use]
    pub const fn new(
        (image_id, protocol): (GraphicsImageId, GraphicsProtocol),
        bytes: ByteCount,
        placement: Option<GraphicsIndexPlacement>,
    ) -> Self {
        Self {
            image_id,
            format: OpenEnum::Known(protocol),
            bytes,
            placement,
        }
    }
}
