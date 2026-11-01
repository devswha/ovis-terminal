//! What a reattach re-sent of a terminal's retained graphics, and what it no longer
//! holds (plan todo 84).
//!
//! A terminal retains, within `graphics_terminal_store_max_bytes`, the graphics
//! sequences whose placement is still on screen, keyed by the raw output offset where
//! each began. A checkpoint paint re-sends them byte-identical in receipt order; the
//! `graphics_replayed` record that follows names them, and names the most recently
//! released ones with the reason each left.

#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::scalar::{ByteOffset, OpenEnum};

/// Why a retained graphics sequence is no longer held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GraphicsReleaseReason {
    /// The row it was placed on left the visible screen.
    ScrolledOut,
    /// The cell it was placed on was erased or overwritten, or its alternate screen
    /// was left.
    Erased,
    /// A Kitty `a=d` command named it.
    Deleted,
    /// A Kitty transmission reused its image id.
    Replaced,
    /// The terminal was reset (RIS).
    Reset,
    /// The terminal retained more images than it tracks; the oldest went first.
    Limit,
    /// Retaining a newer sequence exceeded `graphics_terminal_store_max_bytes`.
    StoreFull,
    /// It is no longer shown; the specific cause was not retained.
    Expired,
}

/// One released graphics sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsReleased {
    /// Raw output offset where the sequence began.
    pub output_offset: ByteOffset,
    /// Why it left.
    pub reason: OpenEnum<GraphicsReleaseReason>,
}

impl GraphicsReleased {
    /// The sequence that began at `output_offset` left for `reason`.
    #[must_use]
    pub const fn new(output_offset: ByteOffset, reason: GraphicsReleaseReason) -> Self {
        Self {
            output_offset,
            reason: OpenEnum::Known(reason),
        }
    }
}

/// Data of a `graphics_replayed` record: the paint before it re-sent `replayed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsReplayedData {
    /// Raw output offsets of the re-sent sequences, in the order the paint sent them.
    pub replayed: Vec<ByteOffset>,
    /// The most recently released sequences, at most 64, in receipt order (by
    /// `output_offset`), however output was split into reads.
    pub released: Vec<GraphicsReleased>,
}

impl GraphicsReplayedData {
    /// The paint re-sent `replayed`; `released` left before it.
    #[must_use]
    pub const fn new(replayed: Vec<ByteOffset>, released: Vec<GraphicsReleased>) -> Self {
        Self { replayed, released }
    }
}
