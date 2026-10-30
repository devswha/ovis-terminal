//! Importing a [`Checkpoint`] into a fresh engine.
//!
//! `alacritty_terminal` exposes only the active grid and handler calls, so the
//! import replays the checkpoint through them in an order where each step
//! overwrites what the previous one clobbered: grids and keyboard stacks per
//! screen first, then terminal-wide state, then the active cursor last.

use alacritty_terminal::grid::{Charsets, Cursor, Dimensions, Grid};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{
    CursorStyle, Handler, KeyboardModes, KeyboardModesApplyBehavior, Mode, NamedMode,
    NamedPrivateMode, PrivateMode, StandardCharset, TabulationClearMode,
};

use crate::engine::Vt;
use crate::engine::capture::CHARSET_SLOTS;
use crate::engine::checkpoint::{CharsetRecord, Checkpoint, CursorRecord, ScreenRecord};
use crate::engine::error::VtError;
use crate::engine::listener::{Inert, Listener};
use crate::engine::modes::{ScreenBuffer, TerminalModes};
use crate::engine::shadow::STACK_DEPTH_LIMIT;
use crate::engine::tracker::{PENDING_LIMIT, Tracker};
use crate::engine::units::Geometry;

const PRIVATE_MODES: [(TermMode, NamedPrivateMode); 13] = [
    (TermMode::SHOW_CURSOR, NamedPrivateMode::ShowCursor),
    (TermMode::APP_CURSOR, NamedPrivateMode::CursorKeys),
    (
        TermMode::MOUSE_REPORT_CLICK,
        NamedPrivateMode::ReportMouseClicks,
    ),
    (
        TermMode::MOUSE_DRAG,
        NamedPrivateMode::ReportCellMouseMotion,
    ),
    (
        TermMode::MOUSE_MOTION,
        NamedPrivateMode::ReportAllMouseMotion,
    ),
    (TermMode::FOCUS_IN_OUT, NamedPrivateMode::ReportFocusInOut),
    (TermMode::BRACKETED_PASTE, NamedPrivateMode::BracketedPaste),
    (TermMode::SGR_MOUSE, NamedPrivateMode::SgrMouse),
    (TermMode::UTF8_MOUSE, NamedPrivateMode::Utf8Mouse),
    (
        TermMode::ALTERNATE_SCROLL,
        NamedPrivateMode::AlternateScroll,
    ),
    (TermMode::LINE_WRAP, NamedPrivateMode::LineWrap),
    (TermMode::ORIGIN, NamedPrivateMode::Origin),
    (TermMode::URGENCY_HINTS, NamedPrivateMode::UrgencyHints),
];

const PUBLIC_MODES: [(TermMode, NamedMode); 2] = [
    (TermMode::INSERT, NamedMode::Insert),
    (TermMode::LINE_FEED_NEW_LINE, NamedMode::LineFeedNewLine),
];

pub(super) fn cursor(record: &CursorRecord) -> Cursor<Cell> {
    let mut charsets = Charsets::default();
    for (slot, charset) in CHARSET_SLOTS.into_iter().zip(record.charsets) {
        charsets[slot] = match charset {
            CharsetRecord::Ascii => StandardCharset::Ascii,
            CharsetRecord::LineDrawing => StandardCharset::SpecialCharacterAndLineDrawing,
        };
    }
    Cursor {
        point: record.point,
        template: record.template.clone(),
        charsets,
        input_needs_wrap: record.input_needs_wrap,
    }
}

fn install(grid: &mut Grid<Cell>, record: &ScreenRecord) {
    *grid = record.grid.clone();
    grid.cursor = cursor(&record.cursor);
    grid.saved_cursor = cursor(&record.saved_cursor);
}

fn push_stack(term: &mut Term<Listener>, stack: &[u8]) {
    for &bits in stack {
        term.push_keyboard_mode(KeyboardModes::from_bits_truncate(bits));
    }
}

fn cursor_fits(record: &CursorRecord, geometry: Geometry) -> bool {
    usize::try_from(record.point.line.0).is_ok_and(|line| line < geometry.row_count())
        && record.point.column.0 < geometry.column_count()
}

fn screen_fits(record: &ScreenRecord, geometry: Geometry) -> bool {
    record.grid.columns() == geometry.column_count()
        && record.grid.screen_lines() == geometry.row_count()
        && cursor_fits(&record.cursor, geometry)
        && cursor_fits(&record.saved_cursor, geometry)
}

fn validate(checkpoint: &Checkpoint) -> Result<(), VtError> {
    let geometry = checkpoint.config.geometry;
    let shadow = &checkpoint.shadow;
    let checks = [
        (
            screen_fits(&checkpoint.primary, geometry),
            "primary screen does not match the geometry",
        ),
        (
            screen_fits(&checkpoint.alternate, geometry),
            "alternate screen does not match the geometry",
        ),
        (
            shadow.tab_stops.len() == geometry.column_count(),
            "tab stops do not match the columns",
        ),
        (
            shadow.scroll_region.start < shadow.scroll_region.end
                && shadow.scroll_region.end <= geometry.row_count(),
            "scroll region lies outside the screen",
        ),
        (
            [&shadow.keyboard_stack, &shadow.inactive_keyboard_stack]
                .iter()
                .all(|stack| stack.len() <= STACK_DEPTH_LIMIT)
                && shadow.title_stack.len() <= STACK_DEPTH_LIMIT,
            "a keyboard or title stack exceeds its depth limit",
        ),
        (
            checkpoint
                .colors
                .iter()
                .all(|(index, _)| usize::from(*index) < COUNT),
            "color index out of range",
        ),
        (
            checkpoint.pending.len() <= PENDING_LIMIT,
            "open sequence exceeds the pending limit",
        ),
        (
            checkpoint.primary.grid.history_size() <= checkpoint.config.history_limit.line_count()
                && checkpoint.alternate.grid.history_size() == 0,
            "history exceeds the configured limit",
        ),
    ];
    checks
        .into_iter()
        .find(|(holds, _)| !holds)
        .map_or(Ok(()), |(_, reason)| {
            Err(VtError::CheckpointInconsistent { reason })
        })
}

/// Entering the alternate screen overwrites the primary saved cursor, so the
/// primary is finalized after leaving it again when the primary is active.
fn install_screens(term: &mut Term<Listener>, checkpoint: &Checkpoint) {
    let shadow = &checkpoint.shadow;
    let (primary_stack, alternate_stack) = match checkpoint.active {
        ScreenBuffer::Primary => (&shadow.keyboard_stack, &shadow.inactive_keyboard_stack),
        ScreenBuffer::Alternate => (&shadow.inactive_keyboard_stack, &shadow.keyboard_stack),
    };
    install(term.grid_mut(), &checkpoint.primary);
    push_stack(term, primary_stack);
    term.swap_alt();
    install(term.grid_mut(), &checkpoint.alternate);
    push_stack(term, alternate_stack);
    if checkpoint.active == ScreenBuffer::Primary {
        term.swap_alt();
        term.grid_mut().saved_cursor = cursor(&checkpoint.primary.saved_cursor);
    }
}

fn install_modes(term: &mut Term<Listener>, mode: TermMode) {
    for (flag, named) in PRIVATE_MODES {
        if mode.contains(flag) {
            term.set_private_mode(PrivateMode::Named(named));
        } else {
            term.unset_private_mode(PrivateMode::Named(named));
        }
    }
    for (flag, named) in PUBLIC_MODES {
        if mode.contains(flag) {
            term.set_mode(Mode::Named(named));
        } else {
            term.unset_mode(Mode::Named(named));
        }
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        term.set_keypad_application_mode();
    } else {
        term.unset_keypad_application_mode();
    }
    let kitty = (mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits() >> 18;
    let kitty = KeyboardModes::from_bits_truncate(u8::try_from(kitty).unwrap_or(u8::MAX));
    term.set_keyboard_mode(kitty, KeyboardModesApplyBehavior::Replace);
}

fn install_terminal_state(term: &mut Term<Listener>, checkpoint: &Checkpoint) {
    let shadow = &checkpoint.shadow;
    install_modes(term, TermMode::from_bits_truncate(checkpoint.mode_bits));
    term.clear_tabs(TabulationClearMode::All);
    for (column, _) in shadow
        .tab_stops
        .iter()
        .enumerate()
        .filter(|(_, stop)| **stop)
    {
        term.goto(0, column);
        term.set_horizontal_tabstop();
    }
    if shadow.scroll_region != (0..checkpoint.config.geometry.row_count()) {
        let region = &shadow.scroll_region;
        term.set_scrolling_region(region.start.saturating_add(1), Some(region.end));
    }
    for title in &shadow.title_stack {
        term.set_title(title.clone());
        term.push_title();
    }
    term.set_title(shadow.title.clone());
    term.set_active_charset(shadow.active_charset.into());
    for &(index, rgb) in &checkpoint.colors {
        term.set_color(usize::from(index), rgb);
    }
    let style = CursorStyle::from(checkpoint.cursor_style);
    if style != term.cursor_style() {
        term.set_cursor_style(Some(style));
    }
    let active = match checkpoint.active {
        ScreenBuffer::Primary => &checkpoint.primary,
        ScreenBuffer::Alternate => &checkpoint.alternate,
    };
    term.grid_mut().cursor = cursor(&active.cursor);
}

impl Vt {
    /// Rebuilds an engine from a checkpoint; feeding it the bytes after
    /// [`Checkpoint::offset`] reproduces the engine that wrote it, subject to
    /// [`Checkpoint::fidelity`], which the restored engine keeps reporting.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::CheckpointInconsistent`] when the checkpoint
    /// contradicts its own geometry or bounds.
    pub fn from_checkpoint(checkpoint: &Checkpoint) -> Result<Self, VtError> {
        validate(checkpoint)?;
        let mut vt = Self::new(checkpoint.config);
        install_screens(&mut vt.term, checkpoint);
        install_terminal_state(&mut vt.term, checkpoint);
        drop(vt.listener.drain());
        let inactive = match checkpoint.active {
            ScreenBuffer::Primary => &checkpoint.alternate,
            ScreenBuffer::Alternate => &checkpoint.primary,
        };
        install(&mut vt.inactive, inactive);
        if let Some(character) = checkpoint.shadow.preceding_char {
            let mut encoded = [0; 4];
            vt.processor
                .advance(&mut Inert, character.encode_utf8(&mut encoded).as_bytes());
        }
        vt.tracker = Tracker::resume(&checkpoint.pending);
        vt.processor.advance(&mut Inert, &checkpoint.pending);
        vt.shadow = checkpoint.shadow.clone();
        vt.offset = checkpoint.offset;
        vt.revision = checkpoint.revision;
        vt.mode_revision = checkpoint.mode_revision;
        vt.modes = TerminalModes::capture(*vt.term.mode(), &vt.shadow);
        vt.fidelity = checkpoint.fidelity.clone();
        vt.restored_at = Some(checkpoint.offset);
        Ok(vt)
    }
}
