//! Emulator state `alacritty_terminal` keeps private but a checkpoint must carry.
//!
//! The recorder updates this mirror at the same handler calls that change the
//! emulator, using the same rules, so export never has to mutate the emulator.

use std::ops::Range;

use alacritty_terminal::vte::ansi::CharsetIndex;
use serde::{Deserialize, Serialize};

use crate::engine::modes::ModifyOtherKeys;
use crate::engine::units::Geometry;

/// alacritty bounds its title and keyboard-mode stacks at this depth.
pub(super) const STACK_DEPTH_LIMIT: usize = 4096;

const DEFAULT_TAB_INTERVAL: usize = 8;

/// G0-G3 slot the GL area currently invokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(super) enum CharsetSlot {
    #[default]
    G0,
    G1,
    G2,
    G3,
}

impl From<CharsetIndex> for CharsetSlot {
    fn from(index: CharsetIndex) -> Self {
        match index {
            CharsetIndex::G0 => Self::G0,
            CharsetIndex::G1 => Self::G1,
            CharsetIndex::G2 => Self::G2,
            CharsetIndex::G3 => Self::G3,
        }
    }
}

impl From<CharsetSlot> for CharsetIndex {
    fn from(slot: CharsetSlot) -> Self {
        match slot {
            CharsetSlot::G0 => Self::G0,
            CharsetSlot::G1 => Self::G1,
            CharsetSlot::G2 => Self::G2,
            CharsetSlot::G3 => Self::G3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Shadow {
    pub(super) tab_stops: Vec<bool>,
    pub(super) scroll_region: Range<usize>,
    pub(super) title: Option<String>,
    pub(super) title_stack: Vec<Option<String>>,
    pub(super) keyboard_stack: Vec<u8>,
    pub(super) inactive_keyboard_stack: Vec<u8>,
    pub(super) active_charset: CharsetSlot,
    pub(super) modify_other_keys: ModifyOtherKeys,
    pub(super) win32_input: bool,
    pub(super) synchronized_update: bool,
    pub(super) preceding_char: Option<char>,
}

fn default_tab_stops(columns: usize) -> Vec<bool> {
    (0..columns)
        .map(|column| column.is_multiple_of(DEFAULT_TAB_INTERVAL))
        .collect()
}

impl Shadow {
    pub(super) fn new(geometry: Geometry) -> Self {
        Self {
            tab_stops: default_tab_stops(geometry.column_count()),
            scroll_region: 0..geometry.row_count(),
            title: None,
            title_stack: Vec::new(),
            keyboard_stack: Vec::new(),
            inactive_keyboard_stack: Vec::new(),
            active_charset: CharsetSlot::G0,
            modify_other_keys: ModifyOtherKeys::Off,
            win32_input: false,
            synchronized_update: false,
            preceding_char: None,
        }
    }

    /// RIS: everything returns to power-on state except the REP character,
    /// which lives in the parser, not the terminal.
    pub(super) fn reset(&mut self, geometry: Geometry) {
        let preceding_char = self.preceding_char;
        *self = Self::new(geometry);
        self.preceding_char = preceding_char;
    }

    /// Mirrors `Term::resize`: new columns get default stops, the region resets.
    pub(super) fn resize(&mut self, geometry: Geometry) {
        let mut column = self.tab_stops.len();
        self.tab_stops.resize_with(geometry.column_count(), || {
            let stop = column.is_multiple_of(DEFAULT_TAB_INTERVAL);
            column = column.saturating_add(1);
            stop
        });
        self.scroll_region = 0..geometry.row_count();
    }

    pub(super) fn set_tab_stop(&mut self, column: usize, stop: bool) {
        if let Some(slot) = self.tab_stops.get_mut(column) {
            *slot = stop;
        }
    }

    pub(super) fn clear_tab_stops(&mut self) {
        self.tab_stops.iter_mut().for_each(|stop| *stop = false);
    }

    /// Mirrors `Term::set_scrolling_region` for a 1-based inclusive request.
    pub(super) fn set_scroll_region(&mut self, requested: Range<usize>, rows: usize) {
        if requested.start >= requested.end {
            return;
        }
        let top = requested.start.saturating_sub(1);
        self.scroll_region = top.min(rows)..requested.end.min(rows);
    }

    pub(super) fn push_title(&mut self) {
        if self.title_stack.len() >= STACK_DEPTH_LIMIT {
            self.title_stack.remove(0);
        }
        self.title_stack.push(self.title.clone());
    }

    pub(super) fn pop_title(&mut self) {
        if let Some(title) = self.title_stack.pop() {
            self.title = title;
        }
    }

    pub(super) fn pop_keyboard_modes(&mut self, count: u16) {
        let kept = self.keyboard_stack.len().saturating_sub(usize::from(count));
        self.keyboard_stack.truncate(kept);
    }

    pub(super) const fn swap_screens(&mut self) {
        std::mem::swap(&mut self.keyboard_stack, &mut self.inactive_keyboard_stack);
    }
}
