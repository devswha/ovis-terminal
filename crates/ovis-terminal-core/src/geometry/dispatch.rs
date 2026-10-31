//! A transition whose change event must commit before the holder message exists.

use ovis_terminal_store::OpId;

use super::{GeometryEpoch, GeometryEvent};
use crate::events::{Commit, Cursor, ResourceId};
use crate::registry::TerminalId;

/// A started transition; only committing its `geometry.changed` event leads to the holder.
#[derive(Debug)]
pub struct Dispatch {
    pub(super) terminal: TerminalId,
    pub(super) epoch: GeometryEpoch,
    pub(super) change: GeometryEvent,
}

/// A transition whose change event committed, bound to the receipt of that very commit.
///
/// [`super::GeometryController::apply`] takes it to release the holder message.
#[derive(Debug)]
pub struct Committed {
    pub(super) epoch: GeometryEpoch,
    pub(super) receipt: Cursor,
}

impl Committed {
    /// The receipt the commit of this transition's change event returned.
    pub const fn receipt(&self) -> Cursor {
        self.receipt
    }
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

    /// Hands the change event's durable unit to `commit` and binds the receipt it returns.
    ///
    /// # Errors
    ///
    /// Returns the commit path's error; nothing was applied.
    pub fn commit<E>(
        self,
        op: OpId,
        commit: impl FnOnce(Commit<GeometryEvent>) -> Result<Cursor, E>,
    ) -> Result<Committed, E> {
        let affected = vec![ResourceId::Terminal(self.terminal)];
        let receipt = commit(Commit::new(op, self.change, affected))?;
        Ok(Committed {
            epoch: self.epoch,
            receipt,
        })
    }
}
