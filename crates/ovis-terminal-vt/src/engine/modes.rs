//! Structured terminal modes that drive input encoding (IDEAL-046, GAP-066).

use alacritty_terminal::term::TermMode;
use serde::{Deserialize, Serialize};

use crate::engine::shadow::Shadow;

/// Which screen buffer is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScreenBuffer {
    /// The scrolling primary screen.
    Primary,
    /// The alternate screen of full-screen programs.
    Alternate,
}

/// Encoding of cursor or keypad keys (DECCKM, DECKPAM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeyEncoding {
    /// ANSI/numeric encoding.
    Normal,
    /// Application encoding.
    Application,
}

/// `XTerm` `modifyOtherKeys` level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModifyOtherKeys {
    /// Disabled.
    #[default]
    Off,
    /// Level 1: modified keys except well-known control combinations.
    ExceptWellDefined,
    /// Level 2: every modified key.
    All,
}

impl ModifyOtherKeys {
    pub(super) const fn level(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::ExceptWellDefined => 1,
            Self::All => 2,
        }
    }
}

/// Mouse reporting protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MouseTracking {
    /// No reports.
    Off,
    /// Button presses (1000).
    Click,
    /// Presses and motion while a button is held (1002).
    Drag,
    /// Every motion (1003).
    Motion,
}

/// Mouse report encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MouseEncoding {
    /// Legacy X10 bytes.
    Default,
    /// UTF-8 extended coordinates (1005).
    Utf8,
    /// SGR decimal reports (1006).
    Sgr,
}

/// How pasted text must be framed (DEC 2004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PasteMode {
    /// The program did not request bracketed paste.
    Plain,
    /// Pasted text must be wrapped in `ESC [200~` / `ESC [201~`.
    Bracketed,
}

/// Keyboard protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct KeyboardModes {
    /// Cursor-key encoding (DECCKM).
    pub cursor_keys: KeyEncoding,
    /// Keypad encoding (DECKPAM/DECKPNM).
    pub keypad: KeyEncoding,
    /// Active kitty keyboard protocol flags (bit set as defined by kitty).
    pub kitty_flags: u8,
    /// `XTerm` `modifyOtherKeys` level.
    pub modify_other_keys: ModifyOtherKeys,
    /// Win32 input mode (DEC 9001).
    pub win32_input: bool,
}

/// Mouse reporting state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MouseModes {
    /// Which events are reported.
    pub tracking: MouseTracking,
    /// How reports are encoded.
    pub encoding: MouseEncoding,
    /// Focus in/out reports (1004).
    pub focus_events: bool,
    /// Wheel becomes arrow keys on the alternate screen (1007).
    pub alternate_scroll: bool,
}

/// Screen presentation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ScreenModes {
    /// Active buffer.
    pub buffer: ScreenBuffer,
    /// Cursor visibility (DECTCEM).
    pub cursor_visible: bool,
    /// Origin mode (DECOM).
    pub origin: bool,
    /// Autowrap (DECAWM).
    pub autowrap: bool,
}

/// Text editing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EditingModes {
    /// Insert mode (IRM).
    pub insert: bool,
    /// Linefeed also returns the carriage (LNM).
    pub linefeed_newline: bool,
    /// The program declared a synchronized update (DEC 2026).
    pub synchronized_update: bool,
}

/// Every mode that changes how input must be encoded or output is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TerminalModes {
    /// Keyboard protocols.
    pub keyboard: KeyboardModes,
    /// Mouse reporting.
    pub mouse: MouseModes,
    /// Paste framing.
    pub paste: PasteMode,
    /// Screen presentation.
    pub screen: ScreenModes,
    /// Text editing.
    pub editing: EditingModes,
}

const fn encoding(application: bool) -> KeyEncoding {
    if application {
        KeyEncoding::Application
    } else {
        KeyEncoding::Normal
    }
}

impl TerminalModes {
    pub(super) fn capture(mode: TermMode, shadow: &Shadow) -> Self {
        let kitty = (mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits() >> 18;
        Self {
            keyboard: KeyboardModes {
                cursor_keys: encoding(mode.contains(TermMode::APP_CURSOR)),
                keypad: encoding(mode.contains(TermMode::APP_KEYPAD)),
                kitty_flags: u8::try_from(kitty).unwrap_or(u8::MAX),
                modify_other_keys: shadow.modify_other_keys,
                win32_input: shadow.win32_input,
            },
            mouse: MouseModes {
                tracking: mouse_tracking(mode),
                encoding: mouse_encoding(mode),
                focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
                alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            },
            paste: if mode.contains(TermMode::BRACKETED_PASTE) {
                PasteMode::Bracketed
            } else {
                PasteMode::Plain
            },
            screen: ScreenModes {
                buffer: screen_buffer(mode),
                cursor_visible: mode.contains(TermMode::SHOW_CURSOR),
                origin: mode.contains(TermMode::ORIGIN),
                autowrap: mode.contains(TermMode::LINE_WRAP),
            },
            editing: EditingModes {
                insert: mode.contains(TermMode::INSERT),
                linefeed_newline: mode.contains(TermMode::LINE_FEED_NEW_LINE),
                synchronized_update: shadow.synchronized_update,
            },
        }
    }
}

pub(super) const fn screen_buffer(mode: TermMode) -> ScreenBuffer {
    if mode.contains(TermMode::ALT_SCREEN) {
        ScreenBuffer::Alternate
    } else {
        ScreenBuffer::Primary
    }
}

const fn mouse_tracking(mode: TermMode) -> MouseTracking {
    if mode.contains(TermMode::MOUSE_MOTION) {
        MouseTracking::Motion
    } else if mode.contains(TermMode::MOUSE_DRAG) {
        MouseTracking::Drag
    } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        MouseTracking::Click
    } else {
        MouseTracking::Off
    }
}

const fn mouse_encoding(mode: TermMode) -> MouseEncoding {
    if mode.contains(TermMode::SGR_MOUSE) {
        MouseEncoding::Sgr
    } else if mode.contains(TermMode::UTF8_MOUSE) {
        MouseEncoding::Utf8
    } else {
        MouseEncoding::Default
    }
}
