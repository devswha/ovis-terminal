//! Exporting engine state into a [`Checkpoint`] without mutating the engine.

use alacritty_terminal::grid::{Cursor, Grid};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::vte::ansi::{CharsetIndex, StandardCharset};

use crate::engine::Vt;
use crate::engine::checkpoint::{
    CHECKPOINT_ENGINE, CHECKPOINT_FORMAT, CHECKPOINT_VERSION, CharsetRecord, Checkpoint,
    CursorRecord, ScreenRecord,
};
use crate::engine::fidelity::InexactReason;
use crate::engine::modes::{ScreenBuffer, screen_buffer};

pub(super) const CHARSET_SLOTS: [CharsetIndex; 4] = [
    CharsetIndex::G0,
    CharsetIndex::G1,
    CharsetIndex::G2,
    CharsetIndex::G3,
];

impl CursorRecord {
    pub(super) fn of(cursor: &Cursor<Cell>) -> Self {
        let charsets = CHARSET_SLOTS.map(|slot| match cursor.charsets[slot] {
            StandardCharset::Ascii => CharsetRecord::Ascii,
            StandardCharset::SpecialCharacterAndLineDrawing => CharsetRecord::LineDrawing,
        });
        Self {
            point: cursor.point,
            template: cursor.template.clone(),
            charsets,
            input_needs_wrap: cursor.input_needs_wrap,
        }
    }
}

impl ScreenRecord {
    pub(super) fn of(grid: &Grid<Cell>) -> Self {
        let mut compact = grid.clone();
        // Drops the ring buffer's preallocated rows (up to 1000 per growth)
        // and unrotates it; the logical content is unchanged.
        compact.truncate();
        Self {
            grid: compact,
            cursor: CursorRecord::of(&grid.cursor),
            saved_cursor: CursorRecord::of(&grid.saved_cursor),
        }
    }
}

impl Vt {
    /// Captures the engine at its current offset.
    ///
    /// The checkpoint is exact unless its fidelity lists a reason; an open
    /// sequence at the offset travels in [`Checkpoint::pending_bytes`].
    pub fn export_checkpoint(&self) -> Checkpoint {
        let active = screen_buffer(*self.term.mode());
        let shown = ScreenRecord::of(self.term.grid());
        let hidden = ScreenRecord::of(&self.inactive);
        let (primary, alternate) = match active {
            ScreenBuffer::Primary => (shown, hidden),
            ScreenBuffer::Alternate => (hidden, shown),
        };
        let mut fidelity = self.fidelity.clone();
        let pending = match self.tracker.open_sequence() {
            Ok(bytes) => bytes.to_vec(),
            Err(reason) => {
                fidelity = fidelity.with(reason);
                Vec::new()
            }
        };
        if active == ScreenBuffer::Alternate && primary.saved_cursor != primary.cursor {
            fidelity = fidelity.with(InexactReason::PrimarySavedCursorUnrepresentable);
        }
        let colors = self.term.colors();
        Checkpoint {
            format: CHECKPOINT_FORMAT.to_owned(),
            version: CHECKPOINT_VERSION,
            engine: CHECKPOINT_ENGINE.to_owned(),
            offset: self.offset,
            revision: self.revision,
            mode_revision: self.mode_revision,
            config: self.config,
            active,
            primary,
            alternate,
            mode_bits: self.term.mode().bits(),
            colors: (0..COUNT)
                .filter_map(|index| Some((u16::try_from(index).ok()?, colors[index]?)))
                .collect(),
            cursor_style: self.term.cursor_style().into(),
            shadow: self.shadow.clone(),
            fidelity,
            pending,
        }
    }
}
