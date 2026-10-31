//! Deterministic thin workspace, tab, pane and split-node layout (plan todo 18).

mod arrange;
mod close;
mod edit;
mod error;
mod geometry;
mod launch;
mod model;
mod persist;
mod plan;
mod records;
mod selection;
mod service;
mod structure;
mod tree;
mod unit;

#[cfg(test)]
mod tests;

pub use edit::{CloseScope, Edit, NoOpReason, Target};
pub use error::{GeometryReason, GeometryRejection, LayoutError};
pub use geometry::{EXTENT, PaneGeometry, Rect, SplitGeometry, TabGeometry};
pub use launch::Launcher;
pub use model::{Axis, Child, Layout, Pane, Ratio, Selection, Split, Tab, Workspace};
pub use persist::{LayoutSnapshot, Producer, SessionProducer, SessionSnapshot};
pub use records::{
    Change, LaunchOutcome, LayoutChanged, LayoutEvent, PaneBound, SelectionCause, SelectionChanged,
};
pub use selection::SelectRequest;
pub use service::{Call, EditRequest, Layouts};
