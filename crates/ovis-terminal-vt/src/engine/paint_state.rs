//! Terminal-wide state in the first paint: modes, keyboard protocols, tab
//! stops, scroll region, palette, title, charsets and the final cursor.
//!
//! Order matters: DECSTBM and DECOM home the cursor, so they precede the
//! cursor placement, and the last-column re-print precedes charset
//! designation, autowrap and insert mode, which would change what it prints.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::vte::ansi::{CharsetIndex, CursorShape, StandardCharset};

use crate::engine::Vt;
use crate::engine::capture::CHARSET_SLOTS;
use crate::engine::fidelity::InexactReason;
use crate::engine::paint::Painter;
use crate::engine::paint_rows::has_wrap_flag;
use crate::engine::recorder::WIN32_INPUT_MODE;
use crate::engine::shadow::{CharsetSlot, Shadow};

const DEC_MODES: [(TermMode, u16); 12] = [
    (TermMode::SHOW_CURSOR, 25),
    (TermMode::APP_CURSOR, 1),
    (TermMode::MOUSE_REPORT_CLICK, 1000),
    (TermMode::MOUSE_DRAG, 1002),
    (TermMode::MOUSE_MOTION, 1003),
    (TermMode::FOCUS_IN_OUT, 1004),
    (TermMode::UTF8_MOUSE, 1005),
    (TermMode::SGR_MOUSE, 1006),
    (TermMode::ALTERNATE_SCROLL, 1007),
    (TermMode::URGENCY_HINTS, 1042),
    (TermMode::BRACKETED_PASTE, 2004),
    (TermMode::ORIGIN, 6),
];

const fn switch(on: bool) -> char {
    if on { 'h' } else { 'l' }
}

impl Painter {
    pub(super) fn palette(&mut self, vt: &Vt) {
        let colors = vt.term.colors();
        for index in 0..COUNT {
            let Some(rgb) = colors[index] else { continue };
            let spec = format!("rgb:{:02x}/{:02x}/{:02x}", rgb.r, rgb.g, rgb.b);
            let selector = match index {
                0..=255 => format!("4;{index}"),
                256 => "10".to_owned(),
                257 => "11".to_owned(),
                258 => "12".to_owned(),
                _ => continue,
            };
            self.emit(format_args!("\x1b]{selector};{spec}\x1b\\"));
        }
    }

    fn tab_stops(&mut self, shadow: &Shadow) {
        let default = shadow
            .tab_stops
            .iter()
            .enumerate()
            .all(|(column, stop)| *stop == column.is_multiple_of(8));
        if default {
            return;
        }
        self.out.push_str("\x1b[3g");
        for (column, _) in shadow
            .tab_stops
            .iter()
            .enumerate()
            .filter(|(_, stop)| **stop)
        {
            self.emit(format_args!("\x1b[{}G\x1bH", column.saturating_add(1)));
        }
    }

    fn keyboard(&mut self, vt: &Vt) {
        for bits in &vt.shadow.keyboard_stack {
            self.emit(format_args!("\x1b[>{bits}u"));
        }
        let kitty = vt.modes.keyboard.kitty_flags;
        self.emit(format_args!("\x1b[={kitty};1u"));
        let level = vt.shadow.modify_other_keys.level();
        self.emit(format_args!("\x1b[>4;{level}m"));
        let win32 = switch(vt.shadow.win32_input);
        self.emit(format_args!("\x1b[?{WIN32_INPUT_MODE}{win32}"));
        let keypad = if vt.term.mode().contains(TermMode::APP_KEYPAD) {
            "\x1b="
        } else {
            "\x1b>"
        };
        self.out.push_str(keypad);
    }

    fn cursor_style(&mut self, vt: &Vt) {
        let style = vt.term.cursor_style();
        let steady: u8 = match style.shape {
            CursorShape::Block => 2,
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
            CursorShape::HollowBlock | CursorShape::Hidden => return,
        };
        let code: u8 = if style.blinking {
            steady.saturating_sub(1)
        } else {
            steady
        };
        self.emit(format_args!("\x1b[{code} q"));
    }

    fn charsets(&mut self, vt: &Vt) {
        let cursor = &vt.term.grid().cursor;
        for (slot, designator) in CHARSET_SLOTS.into_iter().zip(['(', ')', '*', '+']) {
            if cursor.charsets[slot] == StandardCharset::SpecialCharacterAndLineDrawing {
                self.emit(format_args!("\x1b{designator}0"));
            }
        }
        if vt.shadow.active_charset == CharsetSlot::from(CharsetIndex::G1) {
            self.out.push('\x0e');
        }
    }

    pub(super) fn terminal_state(&mut self, vt: &Vt) {
        let grid = vt.term.grid();
        let mode = *vt.term.mode();
        let (cursor, saved) = (&grid.cursor, &grid.saved_cursor);
        let region = &vt.shadow.scroll_region;
        // Under DECOM a CUP cannot leave the region, and re-printing a pending
        // wrap clears the row's wrap flag; such cursors are parked with DECSC
        // and restored with DECRC, which costs the program's own saved cursor
        // unless both are the same.
        let escapes_origin = mode.contains(TermMode::ORIGIN)
            && !usize::try_from(cursor.point.line.0).is_ok_and(|line| region.contains(&line));
        let wrapped_pending = cursor.input_needs_wrap && has_wrap_flag(&grid[cursor.point.line]);
        let parked = escapes_origin || wrapped_pending;
        if parked {
            self.park(grid, cursor);
            let same = saved.point == cursor.point
                && saved.template == cursor.template
                && saved.input_needs_wrap == cursor.input_needs_wrap;
            if !same {
                self.lossy = Some(InexactReason::SavedCursorNotPaintable);
            }
        } else {
            self.goto(saved.point.line, saved.point.column.0);
            self.finish_pen(&saved.template);
            self.out.push_str("\x1b7");
        }
        self.tab_stops(&vt.shadow);
        if *region != (0..grid.screen_lines()) {
            self.emit(format_args!(
                "\x1b[{};{}r",
                region.start.saturating_add(1),
                region.end
            ));
        }
        for (flag, number) in DEC_MODES {
            self.emit(format_args!(
                "\x1b[?{number}{}",
                switch(mode.contains(flag))
            ));
        }
        self.keyboard(vt);
        if let Some(title) = &vt.shadow.title {
            self.emit(format_args!("\x1b]2;{title}\x1b\\"));
        }
        self.cursor_style(vt);
        if parked {
            self.out.push_str("\x1b8");
        } else {
            if mode.contains(TermMode::ORIGIN) {
                self.row_origin = i32::try_from(region.start).unwrap_or(0);
            }
            self.cursor(grid, cursor);
        }
        self.charsets(vt);
        self.emit(format_args!(
            "\x1b[?7{}",
            switch(mode.contains(TermMode::LINE_WRAP))
        ));
        self.emit(format_args!(
            "\x1b[4{}",
            switch(mode.contains(TermMode::INSERT))
        ));
        self.emit(format_args!(
            "\x1b[20{}",
            switch(mode.contains(TermMode::LINE_FEED_NEW_LINE))
        ));
        self.finish_pen(&grid.cursor.template);
    }
}
