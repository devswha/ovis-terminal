//! Where a placement sits on the grid, and whether it is still there.
//!
//! A placement covers the rows and columns its header declares, one cell when it
//! declares none, starting at the cursor cell at receipt and clipped to the grid. Every
//! covered cell is marked with a flag bit alacritty never defines: the emulator replaces
//! a cell wholesale when it prints over or erases it, and resets every row it recycles,
//! so a mark disappears exactly when its cell's content is replaced. Paint and reads mask
//! style flags, so the mark is never visible. Each covered row is followed by the address
//! of its cell buffer, which moves with the row through scrolling and region swaps; the
//! placement stays shown while any covered row is visible and no covered cell lost its
//! mark.

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

/// What became of a placement after output moved rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Presence {
    Shown,
    ScrolledOut,
    Erased,
}

/// One covered row: its buffer identity and the visible row it was last found on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tracked {
    id: usize,
    line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Anchor {
    /// Covered rows still on screen, top first; never empty while shown.
    rows: Vec<Tracked>,
    pub column: usize,
    width: usize,
    pub alternate: bool,
}

fn line(index: usize) -> Line {
    Line(i32::try_from(index).unwrap_or(i32::MAX))
}

/// Identity of a visible row: the address of its first cell.
fn row_id(grid: &Grid<Cell>, index: usize) -> Option<usize> {
    (index < grid.screen_lines()).then(|| std::ptr::from_ref(&grid[line(index)][Column(0)]).addr())
}

impl Anchor {
    /// Marks the footprint of `extent` (rows, columns) at the cursor of `placing`.
    pub(super) fn place(placing: &mut Placing<'_>, extent: (u16, u16)) -> Self {
        let grid = &mut *placing.grid;
        let point = grid.cursor.point;
        let top = usize::try_from(point.line.0).unwrap_or(0);
        let column = point.column.0;
        let bottom = top
            .saturating_add(usize::from(extent.0.max(1)))
            .min(grid.screen_lines());
        let width = usize::from(extent.1.max(1));
        let right = column.saturating_add(width).min(grid.columns());
        let mut rows = Vec::new();
        for index in top..bottom {
            for cell in column..right {
                grid[line(index)][Column(cell)].flags.insert(MARK);
            }
            if let Some(id) = row_id(grid, index) {
                rows.push(Tracked { id, line: index });
            }
        }
        Self {
            rows,
            column,
            width,
            alternate: placing.alternate,
        }
    }

    /// The first covered row still on screen: where a paint re-sends the placement.
    pub(super) fn line(&self) -> usize {
        self.rows.first().map_or(0, |row| row.line)
    }

    fn intact(&self, grid: &Grid<Cell>, index: usize) -> bool {
        let right = self.column.saturating_add(self.width).min(grid.columns());
        index < grid.screen_lines()
            && self.column < right
            && (self.column..right).all(|cell| grid[line(index)][Column(cell)].flags.contains(MARK))
    }

    /// Finds the covered rows on the active screen of `grid` and checks their cells.
    pub(super) fn follow(&mut self, grid: &Grid<Cell>) -> Presence {
        let mut found = Vec::new();
        for row in &self.rows {
            let at = std::iter::once(row.line)
                .chain(0..grid.screen_lines())
                .find(|&index| row_id(grid, index) == Some(row.id));
            if let Some(index) = at {
                found.push(Tracked {
                    id: row.id,
                    line: index,
                });
            }
        }
        found.sort_by_key(|row| row.line);
        self.rows = found;
        if self.rows.is_empty() {
            Presence::ScrolledOut
        } else if self.rows.iter().all(|row| self.intact(grid, row.line)) {
            Presence::Shown
        } else {
            Presence::Erased
        }
    }

    /// Whether every covered row is still intact where it was last found; for a screen
    /// that is not active, whose rows output cannot move.
    pub(super) fn still(&self, grid: &Grid<Cell>) -> bool {
        !self.rows.is_empty() && self.rows.iter().all(|row| self.intact(grid, row.line))
    }
}
