//! Key names (`enter`, `up`, `ctrl+c`, `alt+x`, `f5`, `a`) parsed and encoded for the modes.

mod sequence;

use super::error::EncodingError;
use super::modes::ModeSnapshot;

/// Kitty flag that reports every key as an escape code; this runtime has no encoder for it.
const KITTY_REPORT_ALL_KEYS: u8 = 8;

/// Held modifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Mods {
    shift: bool,
    alt: bool,
    ctrl: bool,
}

/// A key a terminal can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// A printable ASCII character, space included.
    Text(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    /// Cursor-style key by its final byte (`A`-`D`, `H`, `F`).
    Cursor(u8),
    /// F1-F4 by their SS3 final byte (`P`-`S`).
    Function(u8),
    /// A `CSI n ~` key by its number.
    Tilde(u8),
}

fn named(name: &str) -> Option<Key> {
    let mut characters = name.chars();
    if let (Some(single), None) = (characters.next(), characters.next()) {
        return single.is_ascii_graphic().then_some(Key::Text(single));
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Text(' '),
        "backslash" => Key::Text('\\'),
        "plus" => Key::Text('+'),
        "up" => Key::Cursor(b'A'),
        "down" => Key::Cursor(b'B'),
        "right" => Key::Cursor(b'C'),
        "left" => Key::Cursor(b'D'),
        "home" => Key::Cursor(b'H'),
        "end" => Key::Cursor(b'F'),
        "insert" => Key::Tilde(2),
        "delete" | "del" => Key::Tilde(3),
        "pageup" => Key::Tilde(5),
        "pagedown" => Key::Tilde(6),
        "f1" => Key::Function(b'P'),
        "f2" => Key::Function(b'Q'),
        "f3" => Key::Function(b'R'),
        "f4" => Key::Function(b'S'),
        "f5" => Key::Tilde(15),
        "f6" => Key::Tilde(17),
        "f7" => Key::Tilde(18),
        "f8" => Key::Tilde(19),
        "f9" => Key::Tilde(20),
        "f10" => Key::Tilde(21),
        "f11" => Key::Tilde(23),
        "f12" => Key::Tilde(24),
        _ => return None,
    })
}

/// Parses `shift+ctrl+alt+<key>`; each modifier at most once.
fn parse(spec: &str) -> Option<(Mods, Key)> {
    let (modifiers, name) = match spec.rsplit_once('+') {
        Some((modifiers, name)) => (Some(modifiers), name),
        None => (None, spec),
    };
    let key = named(name)?;
    let mut mods = Mods::default();
    for modifier in modifiers.into_iter().flat_map(|all| all.split('+')) {
        let slot = match modifier.to_ascii_lowercase().as_str() {
            "shift" => &mut mods.shift,
            "alt" => &mut mods.alt,
            "ctrl" => &mut mods.ctrl,
            _ => return None,
        };
        if *slot {
            return None;
        }
        *slot = true;
    }
    Some((mods, key))
}

/// Encodes every name or none: names are all parsed and encoded before bytes are returned.
pub(super) fn encode(names: &[String], snapshot: &ModeSnapshot) -> Result<Vec<u8>, EncodingError> {
    let chords = names
        .iter()
        .map(|name| parse(name).ok_or_else(|| EncodingError::UnknownKey { key: name.clone() }))
        .collect::<Result<Vec<_>, _>>()?;
    let keyboard = snapshot.modes().keyboard;
    if !chords.is_empty()
        && (keyboard.win32_input || keyboard.kitty_flags & KITTY_REPORT_ALL_KEYS != 0)
    {
        return Err(EncodingError::KeyboardProtocol {
            revision: snapshot.revision(),
        });
    }
    let mut bytes = Vec::new();
    for (name, (mods, key)) in names.iter().zip(chords) {
        let encoded =
            sequence::chord(mods, key, keyboard).ok_or_else(|| EncodingError::KeyUnencodable {
                key: name.clone(),
                revision: snapshot.revision(),
            })?;
        bytes.extend_from_slice(&encoded);
    }
    Ok(bytes)
}
