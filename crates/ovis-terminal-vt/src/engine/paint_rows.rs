//! Painting screen rows: wrap continuation, wide-character spacers that
//! erasures left behind, deferred tabs and cursors that need DECSC parking.
//!
//! Every routine reproduces how alacritty reached the state live, because its
//! write path applies wide-character fixups that a naive repaint would trigger.

use alacritty_terminal::grid::{Cursor, Dimensions, Grid, Row};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};

use crate::engine::paint::Painter;
use crate::engine::sgr::Pen;

impl Painter {
    pub(super) fn screen(&mut self, grid: &Grid<Cell>) {
        let columns = grid.columns();
        let last_row = grid.bottommost_line();
        let held = last_row.0 > 0 && has_wrap_flag(&grid[last_row]);
        if held {
            self.held_row(&grid[last_row], last_row);
        }
        let mut entry = RowEntry::Fresh;
        for line in (0..=last_row.0).map(Line) {
            let row = &grid[line];
            self.line = line.0;
            match entry {
                RowEntry::Fresh => self.goto(line, 0),
                RowEntry::Wrapped => {}
                RowEntry::AfterPlaceholder => self.erase_placeholder(),
            }
            // A continuation row re-prints what the wrap itself wrote.
            let minimum = match entry {
                RowEntry::Fresh => 0,
                RowEntry::Wrapped => 1,
                RowEntry::AfterPlaceholder => 2,
            };
            if held && line == last_row {
                self.cells(row, minimum);
                break;
            }
            let wraps = line < last_row && has_wrap_flag(row);
            let end = if wraps {
                columns
            } else {
                (0..columns)
                    .rev()
                    .find(|&column| !is_blank(&row[Column(column)]))
                    .map_or(0, |last| last.saturating_add(1))
                    .max(minimum)
            };
            self.cells(row, end);
            entry = if wraps {
                self.wrap(row)
            } else {
                RowEntry::Fresh
            };
        }
        self.flush_tabs();
    }

    pub(super) fn flush_tabs(&mut self) {
        for (line, column) in std::mem::take(&mut self.tabs) {
            self.goto(Line(line), column);
            self.out.push('\t');
        }
    }

    /// Saves the final cursor with DECSC for a later DECRC, for cursors a
    /// direct placement cannot reproduce. A pending wrap on a wrapped row is
    /// rebuilt as it happened live: wrap once (setting the row's wrap flag),
    /// repair the cell the wrap wrote, and let DECRC bring the pending wrap
    /// back. A wrapping bottom row wraps in place inside a region excluding it.
    pub(super) fn park(&mut self, grid: &Grid<Cell>, cursor: &Cursor<Cell>) {
        let line = cursor.point.line;
        let bottom = grid.bottommost_line();
        let wrapped = cursor.input_needs_wrap && has_wrap_flag(&grid[line]);
        let held = wrapped && line == bottom && line.0 > 0;
        if held {
            self.emit(format_args!("\x1b[1;{}r", line.0));
        }
        self.cursor(grid, cursor);
        self.finish_pen(&cursor.template);
        self.out.push_str("\x1b7");
        if wrapped {
            self.out.push(' ');
            let target = if line == bottom {
                line
            } else {
                Line(line.0.saturating_add(1))
            };
            self.line = target.0;
            self.goto(target, 0);
            self.cells(&grid[target], 1);
            self.flush_tabs();
        }
        if held {
            self.out.push_str("\x1b[r");
        }
    }

    /// Erases a wrapped wide placeholder with ECH: printing over it would make
    /// the emulator drop the previous row's leading-spacer flag.
    fn erase_placeholder(&mut self) {
        self.column(0);
        self.out.push_str("\x1b[2X");
    }

    /// Wraps into the next row. A row ending in a leading wide-char spacer
    /// wrapped a wide character; printing a wide placeholder with the spacer's
    /// pen recreates that spacer whatever the next row now holds.
    fn wrap(&mut self, row: &Row<Cell>) -> RowEntry {
        let last = row.len().saturating_sub(1);
        if row[Column(last)]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER)
        {
            self.style(&row[Column(last)]);
            self.out.push('\u{4e2d}');
            RowEntry::AfterPlaceholder
        } else {
            RowEntry::Wrapped
        }
    }

    /// The bottom row can carry a wrap flag without a following row (it was
    /// below the scroll region, where wrapping does not scroll). It is painted
    /// first inside a region that excludes it, so the extra character wraps in
    /// place, then its first cells are re-printed and the region reset. A row
    /// ending in a leading wide-char spacer wrapped a wide character, so the
    /// placeholder is wide too and recreates that spacer.
    fn held_row(&mut self, row: &Row<Cell>, line: Line) {
        let Some(last) = row.len().checked_sub(1) else {
            return;
        };
        let leading = row[Column(last)]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER);
        self.emit(format_args!("\x1b[1;{}r", line.0));
        self.line = line.0;
        self.goto(line, 0);
        self.cells(row, row.len());
        self.style(&row[Column(last)]);
        if leading {
            self.out.push('\u{4e2d}');
            self.erase_placeholder();
            self.cells(row, 2);
        } else {
            self.out.push(' ');
            self.column(0);
            self.cells(row, 1);
        }
        self.out.push_str("\x1b[r");
    }

    fn cells(&mut self, row: &Row<Cell>, end: usize) {
        for column in 0..end {
            let cell = &row[Column(column)];
            let after_wide = column
                .checked_sub(1)
                .is_some_and(|left| row[Column(left)].flags.contains(Flags::WIDE_CHAR));
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                if !after_wide {
                    self.orphan_spacer(row, column);
                }
                if cell.c == '\t' {
                    self.tabs.push((self.line, column));
                }
                continue;
            }
            if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                if cell.c == '\t' {
                    self.tabs.push((self.line, column));
                }
                continue;
            }
            self.cell(cell, column);
            let next = column.saturating_add(1);
            let spaced = row
                .into_iter()
                .nth(next)
                .is_none_or(|right| right.flags.contains(Flags::WIDE_CHAR_SPACER));
            if cell.flags.contains(Flags::WIDE_CHAR) && !spaced {
                // Erasing an earlier spacer left this wide char without one;
                // ECH clears the spacer printing created, without the
                // wide-char fixup that printing over it would trigger.
                self.column(next);
                self.style(&Cell::default());
                self.out.push_str("\x1b[X");
            }
        }
    }

    /// A spacer whose wide char was erased: print a placeholder wide char with
    /// the spacer's pen, erase the placeholder (ECH leaves the spacer), then
    /// print the real left cell, which no longer triggers the wide fixup.
    fn orphan_spacer(&mut self, row: &Row<Cell>, column: usize) {
        let Some(left) = column.checked_sub(1) else {
            return;
        };
        self.column(left);
        self.style(&row[Column(column)]);
        self.out.push('\u{4e2d}');
        self.column(left);
        self.style(&Cell::default());
        self.out.push_str("\x1b[X");
        self.cell(&row[Column(left)], left);
        self.column(column.saturating_add(1));
    }
}

/// How the cursor reached the start of the row being painted.
#[derive(Clone, Copy)]
enum RowEntry {
    /// Positioned explicitly.
    Fresh,
    /// Wrapped from the previous row; printing the first cell completes it.
    Wrapped,
    /// A wide placeholder wrapped into the first two cells.
    AfterPlaceholder,
}

pub(super) fn has_wrap_flag(row: &Row<Cell>) -> bool {
    row.len()
        .checked_sub(1)
        .is_some_and(|last| row[Column(last)].flags.contains(Flags::WRAPLINE))
}

fn is_blank(cell: &Cell) -> bool {
    cell.c == ' '
        && !cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        && Pen::of(cell) == Pen::of(&Cell::default())
        && cell.hyperlink().is_none()
        && cell.zerowidth().is_none()
}
