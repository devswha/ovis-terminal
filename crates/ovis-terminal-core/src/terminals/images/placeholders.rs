//! What a screen read reports for the images this store holds (plan todo 85): one
//! placeholder per image the read screen shows, and the fuller placement index. Both name
//! a store entry and its size only; payload bytes never leave the store, and an image the
//! store evicted or never received has neither.

#[cfg(test)]
mod tests;

use ovis_terminal_protocol::domain::graphics::{
    GraphicsBuffer, GraphicsImageId, GraphicsIndexEntry, GraphicsIndexPlacement,
    GraphicsPlaceholder, PlaceholderCells,
};
use ovis_terminal_protocol::schema::scalar::ByteCount;
use ovis_terminal_vt::{ImageSet, OutputOffset, ScreenBuffer, ScreenImage};

use super::ImageStore;

impl ImageStore {
    /// Raw bytes held for the sequence that began at `key`, if it is held.
    fn held(&self, key: OutputOffset) -> Option<ByteCount> {
        let bytes = self.pieces(key)?.iter().fold(0_u64, |sum, piece| {
            sum.saturating_add(u64::try_from(piece.len()).unwrap_or(u64::MAX))
        });
        Some(ByteCount::new(bytes))
    }

    /// The placeholder of every image in `shown` (a screen read's images) this store
    /// holds, in `shown`'s order.
    #[must_use]
    pub fn placeholders(&self, shown: &[ScreenImage]) -> Vec<GraphicsPlaceholder> {
        shown
            .iter()
            .filter_map(|image| {
                Some(GraphicsPlaceholder::new(
                    (GraphicsImageId::new(image.key.get()), image.protocol),
                    PlaceholderCells::new((image.row, image.column), (image.rows, image.columns)),
                    self.held(image.key)?,
                ))
            })
            .collect()
    }

    /// The placement index: every image `retained` names that this store holds, in
    /// receipt order, with where it is placed on either screen.
    #[must_use]
    pub fn index(&self, retained: &ImageSet) -> Vec<GraphicsIndexEntry> {
        retained
            .retained
            .iter()
            .filter_map(|image| {
                let placement = image.place.map(|place| {
                    let buffer = if place.buffer == ScreenBuffer::Alternate {
                        GraphicsBuffer::Alternate
                    } else {
                        GraphicsBuffer::Primary
                    };
                    GraphicsIndexPlacement::new(
                        buffer,
                        PlaceholderCells::new(
                            (place.row, place.column),
                            (place.rows, place.columns),
                        ),
                    )
                });
                Some(GraphicsIndexEntry::new(
                    (GraphicsImageId::new(image.key.get()), image.protocol),
                    self.held(image.key)?,
                    placement,
                ))
            })
            .collect()
    }
}
