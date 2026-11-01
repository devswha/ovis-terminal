//! Header-only recognition: which sequence an introducer starts, and whether a Kitty
//! chunk announces another (`m=1`). Payload bytes are never read.

use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;

const ESC: u8 = 0x1b;
/// iTerm2 inline images: `OSC 1337 ; File=`.
const ITERM2_FILE: &[u8] = b"1337;File=";
/// Longest sixel parameter run (`DCS P1;P2;P3 q`) still classified.
const SIXEL_PARAMS: usize = 64;
/// Longest Kitty control header read for `m=`.
const KITTY_HEADER: usize = 256;

/// What the bytes of a sequence so far say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// More bytes are needed.
    Undecided,
    /// A graphics sequence of this protocol.
    Graphics(GraphicsProtocol),
    /// Any other sequence.
    Other,
}

/// Classifies a sequence from its first bytes, starting with its ESC.
pub(super) fn classify(bytes: &[u8]) -> Verdict {
    match bytes {
        [ESC] | [ESC, b'_'] => Verdict::Undecided,
        [ESC, b'_', b'G', ..] => Verdict::Graphics(GraphicsProtocol::Kitty),
        [ESC, b']', rest @ ..] => iterm2(rest),
        [ESC, b'P', rest @ ..] => sixel(rest),
        _ => Verdict::Other,
    }
}

fn iterm2(rest: &[u8]) -> Verdict {
    if rest.starts_with(ITERM2_FILE) {
        Verdict::Graphics(GraphicsProtocol::Iterm2)
    } else if ITERM2_FILE.starts_with(rest) {
        Verdict::Undecided
    } else {
        Verdict::Other
    }
}

/// `DCS` parameters (digits, `;`, `:`) then the final `q`; an intermediate such as
/// DECRQSS's `$` makes it another sequence.
fn sixel(rest: &[u8]) -> Verdict {
    let params = rest
        .iter()
        .take_while(|byte| (0x30..=0x3b).contains(*byte))
        .count();
    match rest.get(params) {
        Some(b'q') => Verdict::Graphics(GraphicsProtocol::Sixel),
        None if params < SIXEL_PARAMS => Verdict::Undecided,
        _ => Verdict::Other,
    }
}

/// The control keys of one Kitty chunk, read up to the `;` before its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum KittyHeader {
    Reading(Vec<u8>),
    Read { more: bool },
}

impl Default for KittyHeader {
    fn default() -> Self {
        Self::Reading(Vec::new())
    }
}

impl KittyHeader {
    pub(super) fn feed(&mut self, byte: u8) {
        let Self::Reading(keys) = self else {
            return;
        };
        if byte == b';' || byte < 0x20 {
            *self = Self::Read {
                more: announces(keys),
            };
        } else if keys.len() < KITTY_HEADER {
            keys.push(byte);
        } else {
            *self = Self::Read { more: false };
        }
    }

    /// Whether this chunk announced that another chunk of the same image follows.
    pub(super) fn more(&self) -> bool {
        match self {
            Self::Reading(keys) => announces(keys),
            Self::Read { more } => *more,
        }
    }
}

fn announces(keys: &[u8]) -> bool {
    keys.split(|byte| *byte == b',').any(|pair| pair == b"m=1")
}
