//! SGR encoding of a cell's style, the inverse of vte's SGR parsing.

use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};

const STYLE_FLAGS: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

const FLAG_CODES: [(Flags, &str); 11] = [
    (Flags::BOLD, "1"),
    (Flags::DIM, "2"),
    (Flags::ITALIC, "3"),
    (Flags::UNDERLINE, "4"),
    (Flags::DOUBLE_UNDERLINE, "4:2"),
    (Flags::UNDERCURL, "4:3"),
    (Flags::DOTTED_UNDERLINE, "4:4"),
    (Flags::DASHED_UNDERLINE, "4:5"),
    (Flags::INVERSE, "7"),
    (Flags::HIDDEN, "8"),
    (Flags::STRIKEOUT, "9"),
];

#[derive(Clone, Copy)]
enum Layer {
    Foreground,
    Background,
    Underline,
}

/// Palette slot of the sixteen ANSI colors SGR 30-37/90-97 can name.
const fn ansi_index(color: NamedColor) -> Option<u8> {
    match color {
        NamedColor::Black => Some(0),
        NamedColor::Red => Some(1),
        NamedColor::Green => Some(2),
        NamedColor::Yellow => Some(3),
        NamedColor::Blue => Some(4),
        NamedColor::Magenta => Some(5),
        NamedColor::Cyan => Some(6),
        NamedColor::White => Some(7),
        NamedColor::BrightBlack => Some(8),
        NamedColor::BrightRed => Some(9),
        NamedColor::BrightGreen => Some(10),
        NamedColor::BrightYellow => Some(11),
        NamedColor::BrightBlue => Some(12),
        NamedColor::BrightMagenta => Some(13),
        NamedColor::BrightCyan => Some(14),
        NamedColor::BrightWhite => Some(15),
        // SGR cannot name these; cells only hold them as the defaults below.
        NamedColor::Foreground
        | NamedColor::Background
        | NamedColor::Cursor
        | NamedColor::DimBlack
        | NamedColor::DimRed
        | NamedColor::DimGreen
        | NamedColor::DimYellow
        | NamedColor::DimBlue
        | NamedColor::DimMagenta
        | NamedColor::DimCyan
        | NamedColor::DimWhite
        | NamedColor::BrightForeground
        | NamedColor::DimForeground => None,
    }
}

fn color_code(color: Color, layer: Layer) -> String {
    let (base, bright, extended, default) = match layer {
        Layer::Foreground => (30_u8, 90_u8, "38", "39"),
        Layer::Background => (40, 100, "48", "49"),
        Layer::Underline => (0, 0, "58", "59"),
    };
    match color {
        Color::Named(named) => match (ansi_index(named), layer) {
            (Some(index), Layer::Underline) => format!("{extended};5;{index}"),
            (Some(index @ 0..=7), _) => base.saturating_add(index).to_string(),
            (Some(index), _) => bright.saturating_add(index.saturating_sub(8)).to_string(),
            (None, _) => default.to_owned(),
        },
        Color::Indexed(index) => format!("{extended};5;{index}"),
        Color::Spec(rgb) => format!("{extended};2;{};{};{}", rgb.r, rgb.g, rgb.b),
    }
}

/// The SGR-visible part of a cell's style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pen {
    foreground: Color,
    background: Color,
    flags: Flags,
    underline: Option<Color>,
}

impl Pen {
    pub(super) fn of(cell: &Cell) -> Self {
        Self {
            foreground: cell.fg,
            background: cell.bg,
            flags: cell.flags & STYLE_FLAGS,
            underline: cell.underline_color(),
        }
    }

    /// A complete `SGR 0;...` sequence selecting this pen.
    pub(super) fn sgr(&self) -> String {
        let mut codes = vec!["0".to_owned()];
        codes.extend(
            FLAG_CODES
                .iter()
                .filter(|(flag, _)| self.flags.contains(*flag))
                .map(|(_, code)| (*code).to_owned()),
        );
        codes.push(color_code(self.foreground, Layer::Foreground));
        codes.push(color_code(self.background, Layer::Background));
        if let Some(underline) = self.underline {
            codes.push(color_code(underline, Layer::Underline));
        }
        format!("\x1b[{}m", codes.join(";"))
    }
}
