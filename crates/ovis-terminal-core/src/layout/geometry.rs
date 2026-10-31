//! Deterministic integer geometry in millionths of the tab extent.

use serde::{Deserialize, Serialize};

use super::model::{Axis, Child, Ratio, Tab};
use crate::events::EventError;
use crate::registry::{PaneId, SplitId, TabId};

/// Tab width and height in geometry units (parts per million).
pub const EXTENT: u32 = 1_000_000;

/// A rectangle in tab-relative parts per million.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl Rect {
    const FULL: Self = Self {
        x: 0,
        y: 0,
        width: EXTENT,
        height: EXTENT,
    };

    /// Extent along the axis a split divides.
    pub const fn along(self, axis: Axis) -> u32 {
        match axis {
            Axis::Horizontal => self.width,
            Axis::Vertical => self.height,
        }
    }

    fn divide(self, axis: Axis, ratio: Ratio) -> Result<(Self, Self), EventError> {
        let first = first_extent(self.along(axis), ratio.permille())?;
        let rest = self
            .along(axis)
            .checked_sub(first)
            .ok_or(EventError::InvalidRecord)?;
        Ok(match axis {
            Axis::Horizontal => (
                Self {
                    width: first,
                    ..self
                },
                Self {
                    x: self.x.saturating_add(first),
                    width: rest,
                    ..self
                },
            ),
            Axis::Vertical => (
                Self {
                    height: first,
                    ..self
                },
                Self {
                    y: self.y.saturating_add(first),
                    height: rest,
                    ..self
                },
            ),
        })
    }
}

/// First-child extent of a split: `extent * permille / 1000`, rounded down.
pub(super) fn first_extent(extent: u32, permille: u16) -> Result<u32, EventError> {
    let scaled = u64::from(extent)
        .checked_mul(u64::from(permille))
        .ok_or(EventError::LimitExceeded)?
        / 1000;
    u32::try_from(scaled).map_err(|_| EventError::LimitExceeded)
}

/// Where one pane is drawn, and whether a zoom hides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PaneGeometry {
    /// Pane.
    pub pane: PaneId,
    /// Rectangle inside the tab.
    pub rect: Rect,
    /// False while another pane of the tab is zoomed.
    pub visible: bool,
}

/// Rectangle of one split node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SplitGeometry {
    /// Split node.
    pub split: SplitId,
    /// Rectangle the split divides.
    pub rect: Rect,
}

/// Resulting geometry of one tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TabGeometry {
    /// Tab.
    pub tab: TabId,
    /// Every pane, in tree order.
    pub panes: Vec<PaneGeometry>,
    /// Every split node, in tree order.
    pub splits: Vec<SplitGeometry>,
}

impl Tab {
    /// Computes every pane and split rectangle of this tab.
    ///
    /// # Errors
    ///
    /// Returns `invalid_record` for a tree that references a missing split.
    pub fn geometry(&self) -> Result<TabGeometry, EventError> {
        let mut out = TabGeometry {
            tab: self.id,
            panes: Vec::new(),
            splits: Vec::new(),
        };
        let mut pending: Vec<(Child, Rect)> = self
            .root
            .into_iter()
            .map(|root| (root, Rect::FULL))
            .collect();
        while let Some((child, rect)) = pending.pop() {
            match child {
                Child::Pane(pane) => out.panes.push(self.place(pane, rect)),
                Child::Split(id) => {
                    let split = self
                        .splits
                        .iter()
                        .find(|split| split.id == id)
                        .ok_or(EventError::InvalidRecord)?;
                    out.splits.push(SplitGeometry { split: id, rect });
                    let (first, second) = rect.divide(split.axis, split.ratio)?;
                    pending.push((split.second, second));
                    pending.push((split.first, first));
                }
            }
            if pending.len() > self.panes.len().saturating_add(self.splits.len()) {
                return Err(EventError::InvalidRecord);
            }
        }
        Ok(out)
    }

    fn place(&self, pane: PaneId, rect: Rect) -> PaneGeometry {
        match self.zoomed {
            Some(zoomed) if zoomed == pane => PaneGeometry {
                pane,
                rect: Rect::FULL,
                visible: true,
            },
            Some(_) => PaneGeometry {
                pane,
                rect,
                visible: false,
            },
            None => PaneGeometry {
                pane,
                rect,
                visible: true,
            },
        }
    }
}
