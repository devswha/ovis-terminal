//! The retained-image view the engine hands out: what is retained, where it is placed
//! now, and why recently released images left.

use std::collections::BTreeMap;

use alacritty_terminal::grid::Grid;
use alacritty_terminal::term::cell::Cell;
use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;
use serde::{Deserialize, Serialize};

use super::{ImageRelease, Images};
use crate::engine::modes::ScreenBuffer;
use crate::engine::read::ScreenImage;
use crate::engine::units::OutputOffset;

/// Where a retained image is placed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImagePlace {
    /// Screen it is on.
    pub buffer: ScreenBuffer,
    /// Zero-based row its placement starts at on screen now: the row its sequence began
    /// at, or the first covered row still visible once that row scrolled off.
    pub row: u16,
    /// Zero-based column of that cell.
    pub column: u16,
    /// Rows the header says it covers; one when the header does not say.
    pub rows: u16,
    /// Columns the header says it covers; one when the header does not say.
    pub columns: u16,
}

/// One retained graphics sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RetainedImage {
    /// Raw output offset where its sequence, or its first Kitty chunk, began.
    pub key: OutputOffset,
    /// Its protocol.
    pub protocol: GraphicsProtocol,
    /// Its placement; none for a Kitty transmission that displays nothing itself.
    pub place: Option<ImagePlace>,
}

/// Retained images in receipt order, and why recently released ones left.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ImageSet {
    /// Retained images, oldest first.
    pub retained: Vec<RetainedImage>,
    released: BTreeMap<OutputOffset, ImageRelease>,
}

impl ImageSet {
    /// Whether the image whose sequence began at `key` is retained.
    pub fn retains(&self, key: OutputOffset) -> bool {
        self.retained.iter().any(|image| image.key == key)
    }

    /// Why the image whose sequence began at `key` was released, while remembered.
    pub fn release_of(&self, key: OutputOffset) -> Option<ImageRelease> {
        self.released.get(&key).copied()
    }
}

impl Images {
    /// The images shown on `grid`, the active screen (`alternate` when it is the
    /// alternate one), by the cells each still covers, top row first. Every retained
    /// placement on the active screen is intact: each feed and resize ends in
    /// [`Images::follow`], which releases a footprint that is no longer whole.
    pub(in crate::engine) fn on_screen(
        &self,
        grid: &Grid<Cell>,
        alternate: bool,
    ) -> Vec<ScreenImage> {
        let mut shown: Vec<ScreenImage> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let anchor = entry.anchor.as_ref()?;
                (anchor.alternate == alternate).then(|| ScreenImage {
                    key: entry.key,
                    protocol: entry.protocol,
                    row: u16::try_from(anchor.line()).unwrap_or(u16::MAX),
                    column: u16::try_from(anchor.column).unwrap_or(u16::MAX),
                    rows: u16::try_from(anchor.shown_rows()).unwrap_or(u16::MAX),
                    columns: u16::try_from(anchor.shown_columns(grid)).unwrap_or(u16::MAX),
                })
            })
            .collect();
        shown.sort_by_key(|image| (image.row, image.column, image.key));
        shown
    }

    pub(super) fn set(&self) -> ImageSet {
        let retained = self
            .entries
            .iter()
            .map(|entry| RetainedImage {
                key: entry.key,
                protocol: entry.protocol,
                place: entry.anchor.as_ref().map(|anchor| ImagePlace {
                    buffer: if anchor.alternate {
                        ScreenBuffer::Alternate
                    } else {
                        ScreenBuffer::Primary
                    },
                    row: u16::try_from(anchor.line()).unwrap_or(u16::MAX),
                    column: u16::try_from(anchor.column).unwrap_or(u16::MAX),
                    rows: entry.extent.0.max(1),
                    columns: entry.extent.1.max(1),
                }),
            })
            .collect();
        ImageSet {
            retained,
            released: self.released.iter().copied().collect(),
        }
    }
}
