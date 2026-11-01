//! Which retained images a paint re-sends, and where.
//!
//! A placement is re-sent when its anchor is still marked on the painted screen; a Kitty
//! put also needs the transmission it displays, which is re-sent first, at its own place
//! in receipt order. A put whose image only arrived with an `a=T` that is no longer
//! shown has nothing to display and is skipped.

use std::collections::BTreeSet;

use alacritty_terminal::grid::Grid;
use alacritty_terminal::term::cell::Cell;
use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;

use super::{Entry, ImageSplice, Images, Role};
use crate::engine::modes::ScreenBuffer;

impl Images {
    /// Splices re-sending, at paint byte `at`, the images shown on `buffer`, whose
    /// visible rows `grid` holds, in receipt order.
    pub(in crate::engine) fn splices(
        &self,
        at: usize,
        (grid, buffer): (&Grid<Cell>, ScreenBuffer),
    ) -> Vec<ImageSplice> {
        let alternate = buffer == ScreenBuffer::Alternate;
        let shown: BTreeSet<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .anchor
                    .is_some_and(|anchor| anchor.alternate == alternate && anchor.still(grid))
            })
            .map(|(index, _)| index)
            .collect();
        let mut chosen = BTreeSet::new();
        for &index in &shown {
            let Some(entry) = self.entries.get(index) else {
                continue;
            };
            if entry.role != Role::Put {
                chosen.insert(index);
                continue;
            }
            match self.source(index) {
                Some((source, Role::Data)) => {
                    chosen.extend([source, index]);
                }
                Some((source, _)) if shown.contains(&source) => {
                    chosen.insert(index);
                }
                _ => {}
            }
        }
        chosen
            .into_iter()
            .filter_map(|index| self.entries.get(index))
            .map(|entry| splice(entry, at))
            .collect()
    }

    /// The newest Kitty image before entry `index` that its put displays.
    fn source(&self, index: usize) -> Option<(usize, Role)> {
        let put = self.entries.get(index)?.kitty;
        self.entries
            .iter()
            .enumerate()
            .take(index)
            .rev()
            .find(|(_, entry)| {
                entry.protocol == GraphicsProtocol::Kitty
                    && entry.role != Role::Put
                    && entry.kitty.named_by(put)
            })
            .map(|(source, entry)| (source, entry.role))
    }
}

fn splice(entry: &Entry, at: usize) -> ImageSplice {
    let cursor = entry.anchor.map_or_else(Vec::new, |anchor| {
        format!(
            "\x1b[{};{}H",
            anchor.line.saturating_add(1),
            anchor.column.saturating_add(1)
        )
        .into_bytes()
    });
    ImageSplice {
        at,
        key: entry.key,
        cursor,
    }
}
