//! Where a placement sits on the grid, and whether it is still there.
//!
//! The anchor cell is marked with a flag bit alacritty never defines: the emulator
//! replaces a cell wholesale when it prints over or erases it, and resets every row it
//! recycles, so the mark disappears exactly when the cell's content is replaced. Paint
//! and reads mask style flags, so the mark is never visible. The anchor row is followed
//! by the address of its cell buffer, which moves with the row through scrolling and
//! region swaps.

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};

/// The mark: bit 15 of a cell's flags, unused by alacritty 0.26.
pub(super) const MARK: Flags = Flags::from_bits_retain(1 << 15);

/// The grid a sequence arrived on, and which screen it is.
#[derive(Debug)]
pub(in crate::engine) struct Placing<'a> {
    pub grid: &'a mut Grid<Cell>,
    pub alternate: bool,
}

/// What became of an anchor after output moved rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Presence {
    Shown,
    ScrolledOut,
    Erased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Anchor {
    row: usize,
    /// Visible row it was last found on.
    pub line: usize,
    pub column: usize,
    pub alternate: bool,
}

fn line(index: usize) -> Line {
    Line(i32::try_from(index).unwrap_or(i32::MAX))
}

/// Identity of a visible row: the address of its first cell.
fn row_id(grid: &Grid<Cell>, index: usize) -> Option<usize> {
    (index < grid.screen_lines()).then(|| std::ptr::from_ref(&grid[line(index)][Column(0)]).addr())
}

fn marked(grid: &Grid<Cell>, (index, column): (usize, usize)) -> bool {
    index < grid.screen_lines()
        && column < grid.columns()
        && grid[line(index)][Column(column)].flags.contains(MARK)
}

impl Anchor {
    /// Marks the cursor cell of `placing` as the anchor of a new placement.
    pub(super) fn place(placing: &mut Placing<'_>) -> Self {
        let point = placing.grid.cursor.point;
        let index = usize::try_from(point.line.0).unwrap_or(0);
        placing.grid[point.line][point.column].flags.insert(MARK);
        Self {
            row: row_id(placing.grid, index).unwrap_or(0),
            line: index,
            column: point.column.0,
            alternate: placing.alternate,
        }
    }

    /// Finds the anchor row on the active screen of `grid` and checks its cell.
    pub(super) fn follow(&mut self, grid: &Grid<Cell>) -> Presence {
        let found = std::iter::once(self.line)
            .chain(0..grid.screen_lines())
            .find(|&index| row_id(grid, index) == Some(self.row));
        let Some(index) = found else {
            return Presence::ScrolledOut;
        };
        self.line = index;
        if marked(grid, (index, self.column)) {
            Presence::Shown
        } else {
            Presence::Erased
        }
    }

    /// Whether the anchor is still marked where it was last found; for a screen that
    /// is not active, whose rows output cannot move.
    pub(super) fn still(&self, grid: &Grid<Cell>) -> bool {
        marked(grid, (self.line, self.column))
    }
}
