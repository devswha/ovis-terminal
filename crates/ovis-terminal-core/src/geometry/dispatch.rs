//! A transition whose change event must commit before the holder message exists.

use ovis_terminal_store::OpId;

use super::{GeometryEpoch, GeometryEvent};
use crate::events::{Commit, ResourceId};
use crate::registry::TerminalId;

/// A started transition; only committing its `geometry.changed` event leads to the holder.
#[derive(Debug)]
pub struct Dispatch {
    pub(super) terminal: TerminalId,
    pub(super) epoch: GeometryEpoch,
    pub(super) change: GeometryEvent,
}

/// Proof that a transition's change event was handed to the commit path.
///
/// [`super::GeometryController::apply`] takes it together with the commit receipt.
#[derive(Debug)]
pub struct Committing {
    pub(super) epoch: GeometryEpoch,
}

impl Dispatch {
    /// Epoch of the transition.
    pub const fn epoch(&self) -> GeometryEpoch {
        self.epoch
    }

    /// The `geometry.changed` event.
    pub const fn event(&self) -> &GeometryEvent {
        &self.change
    }

    /// The durable unit to commit first, and the token that releases the holder message.
    pub fn into_commit(self, op: OpId) -> (Commit<GeometryEvent>, Committing) {
        let affected = vec![ResourceId::Terminal(self.terminal)];
        let committing = Committing { epoch: self.epoch };
        (Commit::new(op, self.change, affected), committing)
    }
}
