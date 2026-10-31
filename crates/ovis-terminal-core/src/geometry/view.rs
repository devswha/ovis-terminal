//! Reads of the geometry state: input readiness, unchanged-grid repaint and focus.

use ovis_terminal_vt::{FirstPaint, Geometry};

use super::controller::GeometryController;
use super::{GeometryEpoch, GeometryError};
use crate::leases::{Lease, LeaseError, LeaseView, Owner};
use crate::registry::LeaseId;

/// A paint of the current screen at the unchanged grid.
#[derive(Debug)]
#[non_exhaustive]
pub struct Repaint {
    /// Epoch whose grid the paint has.
    pub epoch: GeometryEpoch,
    /// The paint, its offset, revision, modes and cursor.
    pub paint: FirstPaint,
}

/// A focus record; it never moves input or geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Focused {
    /// The focused client's lease.
    pub lease: Lease,
    /// Geometry owner, unchanged by focus.
    pub geometry_owner: Option<Lease>,
}

impl GeometryController {
    /// The grid the VT has.
    pub const fn grid(&self) -> Geometry {
        self.grid
    }

    /// The newest epoch issued.
    pub const fn epoch(&self) -> GeometryEpoch {
        self.issued
    }

    /// The epoch input is ready at; the runtime asserts it, never output silence.
    ///
    /// # Errors
    ///
    /// Returns `not_ready` while a grid change is being applied.
    pub const fn input_ready(&self) -> Result<GeometryEpoch, GeometryError> {
        match &self.flight {
            Some(flight) => Err(GeometryError::NotReady {
                pending: flight.epoch,
            }),
            None => Ok(self.issued),
        }
    }

    /// A coherent paint at the unchanged grid; sends nothing to the holder or the program.
    ///
    /// # Errors
    ///
    /// Returns `not_ready` while a grid change is being applied, or the screen's refusal.
    pub fn repaint(&self) -> Result<Repaint, GeometryError> {
        let epoch = self.input_ready()?;
        let paint = self.screen.first_paint().map_err(GeometryError::Screen)?;
        Ok(Repaint { epoch, paint })
    }

    /// Records that `owner`'s client holding `lease` has focus.
    ///
    /// # Errors
    ///
    /// Returns `no_lease` unless `owner` holds `lease` in `view`.
    pub fn focus(
        &mut self,
        view: &LeaseView,
        claim: (&Owner, LeaseId),
    ) -> Result<Focused, GeometryError> {
        let (owner, lease) = claim;
        let held = view
            .interact
            .iter()
            .chain(&view.geometry)
            .chain(&view.observers)
            .find(|held| held.lease_id == lease && held.owner == *owner)
            .ok_or(GeometryError::Lease(LeaseError::NoLease))?;
        self.focused = Some(held.lease_id);
        Ok(Focused {
            lease: held.clone(),
            geometry_owner: view.geometry.clone(),
        })
    }

    /// The lease whose client last reported focus.
    pub const fn focused(&self) -> Option<LeaseId> {
        self.focused
    }
}
