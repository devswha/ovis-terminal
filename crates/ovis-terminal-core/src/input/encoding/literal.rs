//! Literal text: refused when it would act, framed only where the program asked for it.

use ovis_terminal_protocol::schema::methods::input::BracketedPolicy;
use ovis_terminal_vt::PasteMode;

use super::error::{EncodingError, PayloadIndex};
use super::modes::ModeSnapshot;

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
/// Seven-bit and eight-bit (C1 CSI) forms of both markers.
const MARKERS: [&str; 4] = ["\x1b[200~", "\x1b[201~", "\u{9b}200~", "\u{9b}201~"];

/// How the literal is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Framing {
    Plain,
    Bracketed,
}

/// Who decides the framing: a typed send or a paste policy.
#[derive(Debug, Clone, Copy)]
pub(super) enum Intent {
    Type,
    Paste(BracketedPolicy),
}

/// Whether `text` holds line breaks or tabs; every other control is refused.
fn structured(text: &str) -> Result<bool, EncodingError> {
    if let Some(at) = MARKERS.iter().filter_map(|marker| text.find(marker)).min() {
        return Err(EncodingError::PasteMarker {
            at: PayloadIndex::new(at),
        });
    }
    let mut structured = false;
    for (at, character) in text.char_indices() {
        match character {
            '\n' | '\r' | '\t' => structured = true,
            control if control.is_control() => {
                return Err(EncodingError::ControlCharacter {
                    at: PayloadIndex::new(at),
                    code: u32::from(control),
                });
            }
            _ => {}
        }
    }
    Ok(structured)
}

/// Decides the framing of `text` for `intent` under the snapshot's paste mode.
pub(super) fn frame(
    text: &str,
    intent: Intent,
    snapshot: &ModeSnapshot,
) -> Result<Framing, EncodingError> {
    let structured = structured(text)?;
    let bracketed = matches!(snapshot.modes().paste, PasteMode::Bracketed);
    let refused = EncodingError::NeedsBracketedPaste {
        revision: snapshot.revision(),
    };
    let framing = match intent {
        Intent::Type if structured && bracketed => Framing::Bracketed,
        Intent::Paste(BracketedPolicy::Auto | BracketedPolicy::Require) if bracketed => {
            Framing::Bracketed
        }
        Intent::Paste(BracketedPolicy::Require) => return Err(refused),
        Intent::Type | Intent::Paste(BracketedPolicy::Auto | BracketedPolicy::Never) => {
            if structured {
                return Err(refused);
            }
            Framing::Plain
        }
        Intent::Paste(_) => return Err(EncodingError::UnsupportedOption),
    };
    Ok(framing)
}

/// Appends `text` with its framing.
pub(super) fn write(text: &str, framing: Framing, out: &mut Vec<u8>) {
    match framing {
        Framing::Plain => out.extend_from_slice(text.as_bytes()),
        Framing::Bracketed => {
            out.extend_from_slice(PASTE_START);
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(PASTE_END);
        }
    }
}
