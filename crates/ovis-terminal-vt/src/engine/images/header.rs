//! Header-only reading of one completed sequence: its graphics protocol and the few
//! keys that decide where, or whether, it is placed. Payload bytes are never read.
//!
//! The introducer rules are the attach graphics gate's (`streams/graphics/introducer.rs`
//! in the server), so the engine and the gate name every sequence by the same offset.

use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;

const ESC: u8 = 0x1b;
/// iTerm2 inline images: `OSC 1337 ; File=`.
const ITERM2_FILE: &[u8] = b"1337;File=";
/// The gate classifies `DCS ... q` as sixel only below this many parameter bytes.
const SIXEL_PARAMS: usize = 64;
/// The gate reads at most this many Kitty control-key bytes for `m=`.
const KITTY_HEADER: usize = 256;

/// What a completed sequence means for retained images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::engine) enum Class {
    /// A graphics sequence.
    Graphics(Header),
    /// RIS (`ESC c`), which removes every image.
    Reset,
}

/// The header of one graphics sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::engine) enum Header {
    Kitty(Kitty),
    Iterm2(Iterm2),
    Sixel,
}

impl Header {
    pub(super) const fn protocol(self) -> GraphicsProtocol {
        match self {
            Self::Kitty(_) => GraphicsProtocol::Kitty,
            Self::Iterm2(_) => GraphicsProtocol::Iterm2,
            Self::Sixel => GraphicsProtocol::Sixel,
        }
    }
}

/// The control keys of one Kitty chunk that matter here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::engine) struct Kitty {
    /// `a=`: `t` transmit (the default), `T` transmit and display, `p` put, `d` delete.
    pub action: u8,
    /// `d=`: what a delete names; `a` (all) by default.
    pub delete: u8,
    /// `i=`: client-chosen image id.
    pub id: Option<u32>,
    /// `I=`: image number.
    pub number: Option<u32>,
    /// `r=`: rows covered, zero when absent.
    pub rows: u16,
    /// `c=`: columns covered, zero when absent.
    pub columns: u16,
    /// `m=1`: another chunk of this transmission follows.
    pub more: bool,
}

impl Default for Kitty {
    fn default() -> Self {
        Self {
            action: b't',
            delete: b'a',
            id: None,
            number: None,
            rows: 0,
            columns: 0,
            more: false,
        }
    }
}

/// The arguments of one iTerm2 `File=` sequence that matter here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::engine) struct Iterm2 {
    /// `inline=1`: displayed rather than downloaded.
    pub inline: bool,
    /// `height=N` in cells, zero when absent or not in cells.
    pub rows: u16,
    /// `width=N` in cells, zero when absent or not in cells.
    pub columns: u16,
}

/// Classifies a completed sequence from its first bytes, starting with its ESC.
pub(super) fn classify(head: &[u8]) -> Option<Class> {
    match head {
        [ESC, b'c'] => Some(Class::Reset),
        [ESC, b'_', b'G', rest @ ..] => Some(Class::Graphics(Header::Kitty(kitty(rest)))),
        [ESC, b']', rest @ ..] => rest
            .strip_prefix(ITERM2_FILE)
            .map(|args| Class::Graphics(Header::Iterm2(iterm2(args)))),
        [ESC, b'P', rest @ ..] if sixel(rest) => Some(Class::Graphics(Header::Sixel)),
        _ => None,
    }
}

fn number<T: std::str::FromStr>(digits: &[u8]) -> Option<T> {
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// Keys up to the `;` before the payload, read exactly as far as the gate reads them.
fn kitty(rest: &[u8]) -> Kitty {
    let mut header = Kitty::default();
    let Some(end) = rest
        .iter()
        .position(|byte| *byte == b';' || *byte < 0x20)
        .filter(|end| *end <= KITTY_HEADER)
    else {
        return header;
    };
    for pair in rest
        .get(..end)
        .unwrap_or_default()
        .split(|byte| *byte == b',')
    {
        match pair {
            [b'a', b'=', value] => header.action = *value,
            [b'd', b'=', value] => header.delete = *value,
            [b'i', b'=', value @ ..] => header.id = number(value),
            [b'I', b'=', value @ ..] => header.number = number(value),
            [b'r', b'=', value @ ..] => header.rows = number(value).unwrap_or(0),
            [b'c', b'=', value @ ..] => header.columns = number(value).unwrap_or(0),
            b"m=1" => header.more = true,
            _ => {}
        }
    }
    header
}

/// `key=value;...` arguments before the `:` that starts the payload.
fn iterm2(args: &[u8]) -> Iterm2 {
    let mut image = Iterm2::default();
    let Some(end) = args.iter().position(|byte| *byte == b':') else {
        return image;
    };
    for pair in args
        .get(..end)
        .unwrap_or_default()
        .split(|byte| *byte == b';')
    {
        match pair {
            b"inline=1" => image.inline = true,
            [b'w', b'i', b'd', b't', b'h', b'=', value @ ..] => {
                image.columns = number(value).unwrap_or(0);
            }
            [b'h', b'e', b'i', b'g', b'h', b't', b'=', value @ ..] => {
                image.rows = number(value).unwrap_or(0);
            }
            _ => {}
        }
    }
    image
}

/// `DCS` parameters (digits, `;`, `:`) then the final `q`; an intermediate such as
/// DECRQSS's `$` makes it another sequence.
fn sixel(rest: &[u8]) -> bool {
    let params = rest
        .iter()
        .take_while(|byte| (0x30..=0x3b).contains(*byte))
        .count();
    params < SIXEL_PARAMS && rest.get(params) == Some(&b'q')
}
