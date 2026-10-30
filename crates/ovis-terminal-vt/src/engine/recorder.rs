//! Handler that forwards every parsed action to the emulator and mirrors the
//! private state a checkpoint needs into [`Shadow`].

use alacritty_terminal::grid::Grid;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::cursor_icon::CursorIcon;
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, ClearMode, Color, CursorShape, CursorStyle, Handler, Hyperlink,
    KeyboardModes, KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys,
    NamedPrivateMode, PrivateMode, Rgb, ScpCharPath, ScpUpdateMode, StandardCharset,
    TabulationClearMode,
};

use crate::engine::listener::Listener;
use crate::engine::modes::ModifyOtherKeys as Level;
use crate::engine::shadow::{STACK_DEPTH_LIMIT, Shadow};
use crate::engine::units::Geometry;

/// DEC private mode number of the Win32 input protocol, unknown to alacritty.
pub(super) const WIN32_INPUT_MODE: u16 = 9001;

pub(super) struct Recorder<'a> {
    pub(super) term: &'a mut Term<Listener>,
    pub(super) shadow: &'a mut Shadow,
    /// The screen buffer alacritty keeps out of reach while the other is active.
    pub(super) inactive: &'a mut Grid<Cell>,
    pub(super) geometry: Geometry,
    /// Replies this engine answers itself because alacritty does not.
    pub(super) replies: &'a mut Vec<Vec<u8>>,
}

impl Recorder<'_> {
    fn cursor_column(&self) -> usize {
        self.term.grid().cursor.point.column.0
    }

    fn is_alternate(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Applies a 1049 change and mirrors the buffer that becomes inactive,
    /// including the primary cursor save alacritty performs on entry.
    fn switch_screens(&mut self, apply: impl FnOnce(&mut Term<Listener>)) {
        let was_alternate = self.is_alternate();
        let outgoing = self.term.grid().clone();
        apply(self.term);
        if was_alternate == self.is_alternate() {
            return;
        }
        *self.inactive = outgoing;
        if !was_alternate {
            self.inactive.saved_cursor = self.inactive.cursor.clone();
        }
        self.shadow.swap_screens();
    }

    fn track_private_mode(&mut self, mode: PrivateMode, enabled: bool) {
        match mode {
            PrivateMode::Unknown(WIN32_INPUT_MODE) => self.shadow.win32_input = enabled,
            PrivateMode::Named(NamedPrivateMode::SyncUpdate) => {
                self.shadow.synchronized_update = enabled;
            }
            PrivateMode::Named(NamedPrivateMode::ColumnMode) => {
                self.shadow.scroll_region = 0..self.geometry.row_count();
            }
            PrivateMode::Named(_) | PrivateMode::Unknown(_) => {}
        }
    }
}

/// Forwards handler methods that change no mirrored state. vte fixes these
/// signatures, including the four-parameter `dynamic_color_sequence`.
macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) { Handler::$name(self.term, $($arg),*) })*
    };
}

impl Handler for Recorder<'_> {
    forward! {
        set_cursor_style(style: Option<CursorStyle>);
        set_cursor_shape(shape: CursorShape);
        goto(line: i32, column: usize);
        goto_line(line: i32);
        goto_col(column: usize);
        insert_blank(count: usize);
        move_up(count: usize);
        move_down(count: usize);
        identify_terminal(intermediate: Option<char>);
        device_status(argument: usize);
        move_forward(count: usize);
        move_backward(count: usize);
        move_down_and_cr(count: usize);
        move_up_and_cr(count: usize);
        put_tab(count: u16);
        backspace();
        carriage_return();
        linefeed();
        bell();
        substitute();
        newline();
        scroll_up(count: usize);
        scroll_down(count: usize);
        insert_blank_lines(count: usize);
        delete_lines(count: usize);
        erase_chars(count: usize);
        delete_chars(count: usize);
        move_backward_tabs(count: u16);
        move_forward_tabs(count: u16);
        save_cursor_position();
        restore_cursor_position();
        clear_line(mode: LineClearMode);
        clear_screen(mode: ClearMode);
        set_tabs(interval: u16);
        reverse_index();
        terminal_attribute(attribute: Attr);
        set_mode(mode: Mode);
        unset_mode(mode: Mode);
        report_mode(mode: Mode);
        set_keypad_application_mode();
        unset_keypad_application_mode();
        configure_charset(index: CharsetIndex, charset: StandardCharset);
        set_color(index: usize, color: Rgb);
        dynamic_color_sequence(prefix: String, index: usize, terminator: &str);
        reset_color(index: usize);
        clipboard_store(clipboard: u8, data: &[u8]);
        clipboard_load(clipboard: u8, terminator: &str);
        decaln();
        text_area_size_pixels();
        text_area_size_chars();
        set_hyperlink(hyperlink: Option<Hyperlink>);
        set_mouse_cursor_icon(icon: CursorIcon);
        report_keyboard_mode();
        set_keyboard_mode(mode: KeyboardModes, behavior: KeyboardModesApplyBehavior);
        set_scp(path: ScpCharPath, update: ScpUpdateMode);
    }

    fn input(&mut self, character: char) {
        self.shadow.preceding_char = Some(character);
        self.term.input(character);
    }

    fn set_title(&mut self, title: Option<String>) {
        self.shadow.title.clone_from(&title);
        self.term.set_title(title);
    }

    fn push_title(&mut self) {
        self.shadow.push_title();
        self.term.push_title();
    }

    fn pop_title(&mut self) {
        self.shadow.pop_title();
        self.term.pop_title();
    }

    fn set_horizontal_tabstop(&mut self) {
        self.shadow.set_tab_stop(self.cursor_column(), true);
        self.term.set_horizontal_tabstop();
    }

    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        match mode {
            TabulationClearMode::Current => self.shadow.set_tab_stop(self.cursor_column(), false),
            TabulationClearMode::All => self.shadow.clear_tab_stops(),
        }
        self.term.clear_tabs(mode);
    }

    fn reset_state(&mut self) {
        if self.is_alternate() {
            *self.inactive = self.term.grid().clone();
        }
        self.term.reset_state();
        self.inactive.reset::<Color>();
        self.shadow.reset(self.geometry);
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        let rows = self.geometry.row_count();
        self.shadow
            .set_scroll_region(top..bottom.unwrap_or(rows), rows);
        self.term.set_scrolling_region(top, bottom);
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        self.track_private_mode(mode, true);
        self.switch_screens(|term| term.set_private_mode(mode));
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        self.track_private_mode(mode, false);
        self.switch_screens(|term| term.unset_private_mode(mode));
    }

    fn report_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Unknown(WIN32_INPUT_MODE) {
            let state = if self.shadow.win32_input { 1 } else { 2 };
            self.replies
                .push(format!("\x1b[?{WIN32_INPUT_MODE};{state}$y").into_bytes());
        } else {
            self.term.report_private_mode(mode);
        }
    }

    fn set_active_charset(&mut self, index: CharsetIndex) {
        self.shadow.active_charset = index.into();
        self.term.set_active_charset(index);
    }

    fn push_keyboard_mode(&mut self, mode: KeyboardModes) {
        // alacritty 0.26 evicts from the title stack (and panics when it is
        // empty) once this stack is full; refusing the push keeps the core alive.
        if self.shadow.keyboard_stack.len() >= STACK_DEPTH_LIMIT {
            return;
        }
        self.shadow.keyboard_stack.push(mode.bits());
        self.term.push_keyboard_mode(mode);
    }

    fn pop_keyboard_modes(&mut self, count: u16) {
        self.shadow.pop_keyboard_modes(count);
        self.term.pop_keyboard_modes(count);
    }

    fn set_modify_other_keys(&mut self, mode: ModifyOtherKeys) {
        self.shadow.modify_other_keys = match mode {
            ModifyOtherKeys::Reset => Level::Off,
            ModifyOtherKeys::EnableExceptWellDefined => Level::ExceptWellDefined,
            ModifyOtherKeys::EnableAll => Level::All,
        };
    }

    fn report_modify_other_keys(&mut self) {
        let level = self.shadow.modify_other_keys.level();
        self.replies.push(format!("\x1b[>4;{level}m").into_bytes());
    }
}
