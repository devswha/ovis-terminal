//! Ordered geometry events and the durable units that commit them.

use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::OpId;
use ovis_terminal_vt::{ContentRevision, Geometry, OutputOffset};
use serde::{Deserialize, Serialize};

use super::GeometryEpoch;
use crate::events::{Commit, EventData, ResourceId};
use crate::registry::{LeaseId, TerminalId};

/// One geometry fact, published only after its durable commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum GeometryEvent {
    /// The grid is about to change; committed before the holder or the VT sees it.
    Changed {
        /// Terminal whose grid changes.
        terminal: TerminalId,
        /// Epoch of the change.
        epoch: GeometryEpoch,
        /// New grid.
        grid: Geometry,
        /// Grid it replaces.
        previous: Geometry,
        /// Geometry lease that requested it.
        owner: LeaseId,
        /// Raw output offset at the commit; output at the new grid starts at or after it.
        from_offset: OutputOffset,
    },
    /// The VT applied the grid and the holder acknowledged it: a coherent checkpoint.
    Checkpoint {
        /// Terminal.
        terminal: TerminalId,
        /// Epoch the checkpoint completes.
        epoch: GeometryEpoch,
        /// Grid of the checkpoint.
        grid: Geometry,
        /// Raw output offset the checkpoint reflects.
        offset: OutputOffset,
        /// Content revision the checkpoint reflects.
        revision: ContentRevision,
        /// Committed sequence of the `geometry.changed` event this checkpoint completes.
        changed_seq: EventSeq,
    },
    /// The runtime re-asserts stream-geometry input readiness at this grid.
    InputReady {
        /// Terminal.
        terminal: TerminalId,
        /// Epoch whose completion made input ready.
        epoch: GeometryEpoch,
        /// Grid input is now addressed at.
        grid: Geometry,
        /// Content revision of the completing checkpoint.
        revision: ContentRevision,
    },
}

impl EventData for GeometryEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Changed { .. } => "geometry.changed",
            Self::Checkpoint { .. } => "geometry.checkpoint",
            Self::InputReady { .. } => "geometry.input_ready",
        }
    }
}

/// One durable unit committing `events` in order under `op`, or `None` when there are none.
pub(super) fn commit(
    terminal: TerminalId,
    events: Vec<GeometryEvent>,
    op: OpId,
) -> Option<Commit<GeometryEvent>> {
    let affected = vec![ResourceId::Terminal(terminal)];
    let mut events = events.into_iter();
    let first = events.next()?;
    let unit = Commit::new(op, first, affected.clone());
    Some(events.fold(unit, |unit, event| unit.event(event, affected.clone())))
}
