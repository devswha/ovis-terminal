//! Graphics passthrough caps and the notices that report their enforcement (plan todo 86).
//!
//! The runtime never decodes, validates or transforms an image payload: it measures a
//! whole graphics sequence (a Kitty `APC _G` chunk series, one iTerm2 `OSC 1337 File=`
//! or one sixel `DCS q`) by its raw bytes, keeps or drops it whole, and lets viewers
//! render. The contract and its layer budget are in `docs/design/graphics.md`.

mod replay;
#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::replay::{GraphicsReleaseReason, GraphicsReleased, GraphicsReplayedData};
use crate::schema::limits::Limits;
use crate::schema::scalar::{ByteCount, ByteOffset, Count, OpenEnum};

/// Graphics protocol a sequence belongs to, recognized from its introducer only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GraphicsProtocol {
    /// Kitty graphics: `APC _G ... ST`, one transmission across its `m=1` chunks.
    Kitty,
    /// iTerm2 inline image: `OSC 1337 ; File= ... BEL|ST`.
    Iterm2,
    /// Sixel: `DCS ... q ... ST`.
    Sixel,
}

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

/// Why a whole graphics sequence was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GraphicsDropReason {
    /// The sequence exceeded `graphics_sequence_max_bytes`.
    TooLarge,
}

/// Why retained graphics sequences were evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GraphicsEvictReason {
    /// Retaining a new sequence would exceed `graphics_terminal_store_max_bytes`.
    StoreFull,
}

/// Data of `graphics.dropped`: one sequence was dropped whole, never delivered in part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsDroppedData {
    /// Protocol of the dropped sequence.
    pub protocol: OpenEnum<GraphicsProtocol>,
    /// Why it was dropped.
    pub reason: OpenEnum<GraphicsDropReason>,
    /// Raw output offset where the sequence began.
    pub output_offset: ByteOffset,
    /// Raw bytes seen before the drop; at least `limit_bytes` plus one.
    pub sequence_bytes: ByteCount,
    /// The cap in force.
    pub limit_bytes: ByteCount,
}

/// Data of `graphics.evicted`: the oldest retained sequences left the terminal's store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsEvictedData {
    /// Why sequences were evicted.
    pub reason: OpenEnum<GraphicsEvictReason>,
    /// Sequences evicted.
    pub evicted_sequences: Count,
    /// Raw bytes evicted.
    pub evicted_bytes: ByteCount,
    /// Raw bytes still retained after the eviction.
    pub retained_bytes: ByteCount,
    /// The cap in force.
    pub limit_bytes: ByteCount,
}

impl GraphicsEvictedData {
    /// `evicted_sequences` totalling `evicted_bytes` left a store now holding
    /// `retained_bytes` under `caps`.
    #[must_use]
    pub const fn store_full(
        caps: GraphicsCaps,
        evicted: (Count, ByteCount),
        retained_bytes: ByteCount,
    ) -> Self {
        Self {
            reason: OpenEnum::Known(GraphicsEvictReason::StoreFull),
            evicted_sequences: evicted.0,
            evicted_bytes: evicted.1,
            retained_bytes,
            limit_bytes: caps.terminal_store_max_bytes,
        }
    }
}

/// Outcome of measuring one graphics sequence against the per-sequence cap.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SequenceAdmission {
    /// The sequence fits; it is delivered and retained byte-identical.
    Admitted,
    /// The sequence is dropped whole and this notice is published.
    Dropped(GraphicsDroppedData),
}

/// The two advertised graphics caps, read from the runtime [`Limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GraphicsCaps {
    /// Largest single sequence.
    pub sequence_max_bytes: ByteCount,
    /// Graphics bytes one terminal retains.
    pub terminal_store_max_bytes: ByteCount,
}

impl GraphicsCaps {
    /// The caps `limits` advertises.
    #[must_use]
    pub const fn advertised(limits: &Limits) -> Self {
        Self {
            sequence_max_bytes: limits.graphics_sequence_max_bytes,
            terminal_store_max_bytes: limits.graphics_terminal_store_max_bytes,
        }
    }

    /// Measures a `protocol` sequence of `sequence_bytes` raw bytes that began at
    /// `output_offset`; the cap itself is admitted, one byte more is dropped.
    #[must_use]
    pub const fn admit(
        self,
        protocol: GraphicsProtocol,
        sequence: (ByteOffset, ByteCount),
    ) -> SequenceAdmission {
        let (output_offset, sequence_bytes) = sequence;
        if sequence_bytes.get() <= self.sequence_max_bytes.get() {
            return SequenceAdmission::Admitted;
        }
        SequenceAdmission::Dropped(GraphicsDroppedData {
            protocol: OpenEnum::Known(protocol),
            reason: OpenEnum::Known(GraphicsDropReason::TooLarge),
            output_offset,
            sequence_bytes,
            limit_bytes: self.sequence_max_bytes,
        })
    }
}
