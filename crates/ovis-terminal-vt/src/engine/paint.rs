//! First paint for a late attach (IDEAL-032): bytes that bring a terminal at
//! power-on state to the engine's visible state, plus the facts they encode.

use alacritty_terminal::grid::{Cursor, Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use serde::{Deserialize, Serialize};

use crate::engine::Vt;
use crate::engine::fidelity::{Fidelity, InexactReason};
use crate::engine::modes::{ScreenBuffer, TerminalModes};
use crate::engine::read::CursorState;
use crate::engine::sgr::Pen;
use crate::engine::units::{ContentRevision, Geometry, ModeRevision, OutputOffset};

/// A late-attach paint and the state it reproduces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FirstPaint {
    /// Terminal bytes; live output after `offset` follows them.
    pub bytes: Vec<u8>,
    /// Output offset the paint reflects.
    pub offset: OutputOffset,
    /// Content revision the paint reflects.
    pub revision: ContentRevision,
    /// Mode revision the paint reflects.
    pub mode_revision: ModeRevision,
    /// Grid size the paint assumes.
    pub geometry: Geometry,
    /// Modes the paint establishes.
    pub modes: TerminalModes,
    /// Cursor the paint leaves.
    pub cursor: CursorState,
    /// Primary-screen history lines available through history reads.
    pub history_lines: u32,
    /// Exactness of the painted state.
    pub fidelity: Fidelity,
}

#[derive(Default)]
pub(super) struct Painter {
    pub(super) out: String,
    pen: Option<Pen>,
    link: Option<Hyperlink>,
    /// First region row while origin mode (DECOM) makes positions relative.
    pub(super) row_origin: i32,
    /// State the paint could not reproduce.
    pub(super) lossy: Option<InexactReason>,
    /// Row being painted, for deferred tabs.
    pub(super) line: i32,
    /// Cells holding a tab, replayed with HT once the screen is drawn: HT only
    /// stores the tab in an empty cell, and replaying it inline would clear a
    /// pending wrap at the last column.
    pub(super) tabs: Vec<(i32, usize)>,
}

impl Painter {
    pub(super) fn emit(&mut self, arguments: std::fmt::Arguments<'_>) {
        self.out.push_str(&std::fmt::format(arguments));
    }

    pub(super) fn goto(&mut self, line: Line, column: usize) {
        let row = line.0.saturating_sub(self.row_origin).saturating_add(1);
        let column = column.saturating_add(1);
        self.emit(format_args!("\x1b[{row};{column}H"));
    }

    pub(super) fn column(&mut self, column: usize) {
        self.emit(format_args!("\x1b[{}G", column.saturating_add(1)));
    }

    pub(super) fn style(&mut self, cell: &Cell) {
        let pen = Pen::of(cell);
        if self.pen.as_ref() != Some(&pen) {
            self.out.push_str(&pen.sgr());
            self.pen = Some(pen);
        }
        let link = cell.hyperlink();
        if link != self.link {
            match &link {
                Some(link) => {
                    self.emit(format_args!("\x1b]8;id={};{}\x1b\\", link.id(), link.uri()));
                }
                None => self.out.push_str("\x1b]8;;\x1b\\"),
            }
            self.link = link;
        }
    }

    /// Prints one cell; a tab cell becomes a styled blank and a deferred HT.
    pub(super) fn cell(&mut self, cell: &Cell, column: usize) {
        self.style(cell);
        if cell.c == '\t' {
            self.out.push(' ');
            self.tabs.push((self.line, column));
            return;
        }
        self.out.push(cell.c);
        self.out.extend(cell.zerowidth().unwrap_or_default());
    }

    /// Places the cursor, re-printing the last-column cell when the cursor
    /// is parked past it so the next character wraps exactly as it would live.
    pub(super) fn cursor(&mut self, grid: &Grid<Cell>, cursor: &Cursor<Cell>) {
        let last = grid.columns().saturating_sub(1);
        if cursor.input_needs_wrap {
            let row = &grid[cursor.point.line];
            let column = if row[Column(last)].flags.contains(Flags::WIDE_CHAR_SPACER) {
                last.saturating_sub(1)
            } else {
                last
            };
            self.goto(cursor.point.line, column);
            self.cell(&row[Column(column)], column);
        } else {
            self.goto(cursor.point.line, cursor.point.column.0);
        }
    }

    pub(super) fn finish_pen(&mut self, template: &Cell) {
        self.pen = None;
        self.style(template);
    }
}

impl Vt {
    /// Builds the late-attach paint for the current state.
    pub fn first_paint(&self) -> FirstPaint {
        let mut painter = Painter::default();
        painter.out.push_str("\x1bc");
        painter.palette(self);
        let active = self.term.grid();
        if self.modes.screen.buffer == ScreenBuffer::Alternate {
            painter.screen(&self.inactive);
            // Entering the alternate screen overwrites the primary saved
            // cursor anyway, so parking it there costs nothing.
            painter.park(&self.inactive, &self.inactive.cursor);
            painter.out.push_str("\x1b8");
            // Entering clears the alternate screen with the primary pen; the
            // alternate content is then painted onto default blanks.
            painter.out.push_str("\x1b[?1049h");
            painter.finish_pen(&Cell::default());
            painter.out.push_str("\x1b[2J");
        }
        painter.screen(active);
        painter.terminal_state(self);
        FirstPaint {
            bytes: painter.out.into_bytes(),
            offset: self.offset,
            revision: self.revision,
            mode_revision: self.mode_revision,
            geometry: self.config.geometry,
            modes: self.modes,
            cursor: self.cursor(),
            history_lines: u32::try_from(active.history_size()).unwrap_or(u32::MAX),
            fidelity: painter.lossy.map_or_else(
                || self.fidelity.clone(),
                |reason| self.fidelity.with(reason),
            ),
        }
    }
}
