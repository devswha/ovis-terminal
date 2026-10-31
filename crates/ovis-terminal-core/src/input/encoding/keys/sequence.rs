//! Bytes of one chord in xterm legacy encoding or kitty disambiguation (flag 1).

use ovis_terminal_vt::{KeyEncoding, KeyboardModes};

use super::{Key, Mods};

const ESC: u8 = 0x1b;
const KITTY_DISAMBIGUATE: u8 = 1;

impl Mods {
    const fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }

    /// The xterm/kitty modifier parameter: one plus the shift(1)/alt(2)/ctrl(4) bits.
    fn parameter(self) -> Option<u8> {
        let bits = u8::from(self.shift) | (u8::from(self.alt) << 1) | (u8::from(self.ctrl) << 2);
        bits.checked_add(1)
    }
}

/// `CSI <number> ; <mods> <final>`, the parameter part dropped when no modifier is held.
fn csi(number: u32, mods: Mods, last: char) -> Option<Vec<u8>> {
    let text = if mods.any() {
        format!("\x1b[{number};{}{last}", mods.parameter()?)
    } else {
        format!("\x1b[{number}{last}")
    };
    Some(text.into_bytes())
}

/// Ctrl applied to a character the legacy encoding can carry.
fn control(character: char) -> Option<u8> {
    let byte = u8::try_from(character).ok()?;
    match byte {
        b'a'..=b'z' | b'A'..=b'Z' => Some(byte & 0x1f),
        b' ' | b'@' | b'2' => Some(0x00),
        b'[' | b'3' => Some(0x1b),
        b'\\' | b'4' => Some(0x1c),
        b']' | b'5' => Some(0x1d),
        b'^' | b'6' => Some(0x1e),
        b'_' | b'/' | b'7' => Some(0x1f),
        b'?' | b'8' => Some(0x7f),
        _ => None,
    }
}

fn alt_prefixed(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        bytes.insert(0, ESC);
    }
    bytes
}

fn text(character: char, mods: Mods, kitty: bool) -> Option<Vec<u8>> {
    let letter = character.is_ascii_alphabetic();
    if kitty && (mods.ctrl || mods.alt) {
        return csi(u32::from(character.to_ascii_lowercase()), mods, 'u');
    }
    let byte = match (mods.ctrl, mods.shift) {
        (true, false) => control(character)?,
        (false, true) if letter => u8::try_from(character.to_ascii_uppercase()).ok()?,
        (false, false) => u8::try_from(character).ok()?,
        (_, true) => return None,
    };
    Some(alt_prefixed(mods.alt, vec![byte]))
}

/// Enter, Tab, Backspace and Escape: one legacy byte, CSI u under kitty when ambiguous.
fn editing(key: Key, mods: Mods, kitty: bool) -> Option<Vec<u8>> {
    let (code, byte) = match key {
        Key::Enter => (13, b'\r'),
        Key::Tab => (9, b'\t'),
        Key::Backspace => (127, 0x7f),
        Key::Escape => (27, ESC),
        Key::Text(_) | Key::Cursor(_) | Key::Function(_) | Key::Tilde(_) => return None,
    };
    if kitty && (mods.any() || key == Key::Escape) {
        return csi(code, mods, 'u');
    }
    let plain = Mods { alt: false, ..mods };
    match (key, plain.shift, plain.ctrl) {
        (_, false, false) => Some(alt_prefixed(mods.alt, vec![byte])),
        (Key::Tab, true, false) if !mods.alt => Some(b"\x1b[Z".to_vec()),
        (Key::Backspace, false, true) if !mods.alt => Some(vec![0x08]),
        _ => None,
    }
}

/// Bytes of `key` with `mods` under `keyboard`, or `None` when the modes cannot carry it.
pub(super) fn chord(mods: Mods, key: Key, keyboard: KeyboardModes) -> Option<Vec<u8>> {
    let kitty = keyboard.kitty_flags & KITTY_DISAMBIGUATE != 0;
    let application = matches!(keyboard.cursor_keys, KeyEncoding::Application);
    match key {
        Key::Text(character) => text(character, mods, kitty),
        Key::Enter | Key::Tab | Key::Backspace | Key::Escape => editing(key, mods, kitty),
        Key::Cursor(last) if application && !mods.any() => Some(vec![ESC, b'O', last]),
        Key::Function(last) if !mods.any() => Some(vec![ESC, b'O', last]),
        Key::Cursor(last) | Key::Function(last) if mods.any() => csi(1, mods, char::from(last)),
        Key::Cursor(last) | Key::Function(last) => Some(vec![ESC, b'[', last]),
        Key::Tilde(number) => csi(u32::from(number), mods, '~'),
    }
}
