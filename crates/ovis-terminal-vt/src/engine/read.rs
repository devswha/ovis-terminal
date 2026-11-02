//! Non-mutating reads (IDEAL-035, IDEAL-036): every read takes `&self`, so it
//! cannot scroll, inject input or move the viewport.

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;
use serde::{Deserialize, Serialize};

use crate::engine::Vt;
use crate::engine::checkpoint::CursorStyleRecord;
use crate::engine::fidelity::Fidelity;
use crate::engine::modes::{ScreenBuffer, TerminalModes};
use crate::engine::units::{ContentRevision, Geometry, ModeRevision, OutputOffset};

/// Where read data comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum ReadSource {
    /// The emulator fed since offset zero.
    Emulator,
    /// The emulator restored from a checkpoint at this offset.
    RestoredEmulator {
        /// Offset of the checkpoint.
        checkpoint_offset: OutputOffset,
    },
}

/// Representation of returned lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadFormat {
    /// Plain text, wide-character spacers removed, trailing blanks trimmed.
    Text,
}

/// Cursor shape as the program requested it (DECSCUSR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CursorShape {
    /// Block.
    Block,
    /// Underline.
    Underline,
    /// Vertical bar.
    Beam,
    /// Outlined block.
    HollowBlock,
    /// Invisible.
    Hidden,
}

/// One line of text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ScreenLine {
    /// Line text.
    pub text: String,
    /// The line continues on the next one (soft wrap).
    pub wrapped: bool,
}

/// Cursor state of the active screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CursorState {
    /// Zero-based row.
    pub row: u16,
    /// Zero-based column.
    pub column: u16,
    /// DECTCEM visibility.
    pub visible: bool,
    /// The next printed character wraps first (cursor parked past the last column).
    pub pending_wrap: bool,
    /// Requested shape.
    pub shape: CursorShape,
    /// Requested blinking.
    pub blinking: bool,
}

/// One retained image shown on the active screen, by the cells it covers there. Its
/// cells read as blank text in [`ScreenRead::lines`]; this is what covers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ScreenImage {
    /// Raw output offset where its sequence, or its first Kitty chunk, began: the key
    /// the terminal's image store holds its bytes under.
    pub key: OutputOffset,
    /// Its protocol.
    pub protocol: GraphicsProtocol,
    /// Zero-based row of the first covered row still visible.
    pub row: u16,
    /// Zero-based column of the first covered cell.
    pub column: u16,
    /// Covered rows still visible.
    pub rows: u16,
    /// Covered columns, clipped at the right edge of the grid.
    pub columns: u16,
}

/// The visible screen with every fact a client needs to trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ScreenRead {
    /// Data source.
    pub source: ReadSource,
    /// Line format.
    pub format: ReadFormat,
    /// Output offset the read reflects.
    pub offset: OutputOffset,
    /// Content revision the read reflects.
    pub revision: ContentRevision,
    /// Mode revision the read reflects.
    pub mode_revision: ModeRevision,
    /// Grid size.
    pub geometry: Geometry,
    /// Active buffer.
    pub buffer: ScreenBuffer,
    /// Every visible row, top to bottom.
    pub lines: Vec<ScreenLine>,
    /// Cursor.
    pub cursor: CursorState,
    /// Structured modes.
    pub modes: TerminalModes,
    /// Window title.
    pub title: Option<String>,
    /// Exactness of the underlying state.
    pub fidelity: Fidelity,
    /// Retained images shown on the active screen, top row first.
    #[serde(default)]
    pub images: Vec<ScreenImage>,
}

pub(super) fn row_text(grid: &Grid<Cell>, line: Line) -> ScreenLine {
    let row = &grid[line];
    let mut text = String::new();
    let columns = grid.columns();
    for column in 0..columns {
        let cell = &row[Column(column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        text.push(cell.c);
        text.extend(cell.zerowidth().unwrap_or_default());
    }
    let wrapped = columns
        .checked_sub(1)
        .is_some_and(|last| row[Column(last)].flags.contains(Flags::WRAPLINE));
    ScreenLine {
        text: text.trim_end_matches(' ').to_owned(),
        wrapped,
    }
}

pub(super) fn screen_lines(grid: &Grid<Cell>) -> impl Iterator<Item = ScreenLine> + '_ {
    (0..grid.screen_lines())
        .filter_map(|row| i32::try_from(row).ok())
        .map(|row| row_text(grid, Line(row)))
}

impl Vt {
    pub(super) fn source(&self) -> ReadSource {
        self.restored_at
            .map_or(ReadSource::Emulator, |checkpoint_offset| {
                ReadSource::RestoredEmulator { checkpoint_offset }
            })
    }

    /// Cursor of the active screen.
    pub fn cursor(&self) -> CursorState {
        let cursor = &self.term.grid().cursor;
        let style = self.term.cursor_style();
        let shape = CursorStyleRecord::from(style).shape;
        CursorState {
            row: u16::try_from(cursor.point.line.0).unwrap_or(u16::MAX),
            column: u16::try_from(cursor.point.column.0).unwrap_or(u16::MAX),
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
            pending_wrap: cursor.input_needs_wrap,
            shape,
            blinking: style.blinking,
        }
    }

    /// Reads the visible screen without changing it.
    pub fn screen(&self) -> ScreenRead {
        ScreenRead {
            source: self.source(),
            format: ReadFormat::Text,
            offset: self.offset,
            revision: self.revision,
            mode_revision: self.mode_revision,
            geometry: self.config.geometry,
            buffer: self.modes.screen.buffer,
            lines: screen_lines(self.term.grid()).collect(),
            cursor: self.cursor(),
            modes: self.modes,
            title: self.shadow.title.clone(),
            fidelity: self.fidelity.clone(),
            images: self.images.on_screen(
                self.term.grid(),
                self.term.mode().contains(TermMode::ALT_SCREEN),
            ),
        }
    }
}
