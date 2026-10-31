//! Admission of resize requests under the geometry lease, coalescing and dispatch.

use ovis_terminal_holder::protocol::Resize;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::attach::AttachResizeParams;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_vt::Geometry;

use super::{
    Committing, Dispatch, GeometryEpoch, GeometryError, GeometryEvent, Outcome, Requested,
    Superseded,
};
use crate::events::Cursor;
use crate::leases::{Authorization, LeaseError};
use crate::registry::{LeaseId, TerminalId};
use crate::screen::Screen;

/// The transition the holder and the VT are applying.
#[derive(Debug)]
pub(super) struct Flight {
    pub(super) op: OpId,
    pub(super) epoch: GeometryEpoch,
    pub(super) grid: Geometry,
    /// Sequence of the committed change, set once the VT applied the grid.
    pub(super) changed: Option<EventSeq>,
}

/// The newest request admitted during a transition; rechecked before it is sent.
#[derive(Debug)]
pub(super) struct Queued {
    pub(super) op: OpId,
    pub(super) grid: Geometry,
    pub(super) owner: LeaseId,
    pub(super) admitted: Authorization,
}

/// A request about to be dispatched.
pub(super) struct Next {
    pub(super) op: OpId,
    pub(super) grid: Geometry,
    pub(super) owner: LeaseId,
}

/// Grid, epochs and input readiness of one terminal incarnation.
#[derive(Debug)]
pub struct GeometryController {
    pub(super) terminal: TerminalId,
    pub(super) screen: Screen,
    pub(super) grid: Geometry,
    pub(super) issued: GeometryEpoch,
    pub(super) flight: Option<Flight>,
    pub(super) queued: Option<Queued>,
    pub(super) focused: Option<LeaseId>,
}

impl GeometryController {
    /// Starts ready at the screen's grid, which the holder applied at epoch `applied`.
    ///
    /// # Errors
    ///
    /// Returns the screen's refusal when its state cannot be read.
    pub fn new(
        terminal: TerminalId,
        screen: Screen,
        applied: GeometryEpoch,
    ) -> Result<Self, GeometryError> {
        let grid = screen
            .first_paint()
            .map_err(GeometryError::Screen)?
            .geometry;
        Ok(Self {
            terminal,
            screen,
            grid,
            issued: applied,
            flight: None,
            queued: None,
            focused: None,
        })
    }

    /// Admits a resize the geometry owner was authorized for.
    ///
    /// With no transition in progress a different grid starts one; during a transition
    /// the request waits, replacing (superseding) any older waiting request.
    ///
    /// # Errors
    ///
    /// Returns `invalid_geometry` for a size that is not a grid, `limit_exceeded` when the
    /// epoch cannot advance, or the screen's refusal.
    pub fn request(
        &mut self,
        admitted: Authorization,
        params: &AttachResizeParams,
    ) -> Result<Requested, GeometryError> {
        let (cols, rows) = (params.geometry.cols, params.geometry.rows);
        let grid = Geometry::new(cols, rows).map_err(|source| GeometryError::InvalidGeometry {
            cols,
            rows,
            source,
        })?;
        let owner = admitted
            .lease()
            .map(|lease| lease.lease_id)
            .ok_or(GeometryError::Lease(LeaseError::NoLease))?;
        let op = params.op_id.clone();
        let Some(flight) = &self.flight else {
            let outcome = if grid == self.grid {
                Outcome::Unchanged {
                    grid,
                    epoch: self.issued,
                }
            } else {
                Outcome::Dispatched(self.dispatch(Next { op, grid, owner })?)
            };
            return Ok(Requested {
                outcome,
                superseded: None,
            });
        };
        let (in_flight, epoch) = (flight.grid, flight.epoch);
        let superseded = self.queued.take().map(|older| Superseded {
            op: older.op,
            grid: older.grid,
            by: op.clone(),
        });
        let outcome = if grid == in_flight {
            Outcome::Unchanged { grid, epoch }
        } else {
            self.queued = Some(Queued {
                op,
                grid,
                owner,
                admitted,
            });
            Outcome::Queued { grid }
        };
        Ok(Requested {
            outcome,
            superseded,
        })
    }

    pub(super) fn dispatch(&mut self, next: Next) -> Result<Dispatch, GeometryError> {
        let epoch = self.issued.next()?;
        let from_offset = self.screen.live().map_err(GeometryError::Screen)?.offset;
        let change = GeometryEvent::Changed {
            terminal: self.terminal,
            epoch,
            grid: next.grid,
            previous: self.grid,
            owner: next.owner,
            from_offset,
        };
        self.issued = epoch;
        self.flight = Some(Flight {
            op: next.op,
            epoch,
            grid: next.grid,
            changed: None,
        });
        Ok(Dispatch {
            terminal: self.terminal,
            epoch,
            change,
        })
    }

    /// After the change event committed at `receipt`, applies the grid to the VT and
    /// returns the holder message; nothing at the new grid exists before this call, and a
    /// second call for the same transition is refused.
    ///
    /// # Errors
    ///
    /// Returns `stale_geometry` for a superseded or finished epoch, or the screen's refusal.
    pub fn apply(
        &mut self,
        committing: &Committing,
        receipt: Cursor,
    ) -> Result<Resize, GeometryError> {
        let epoch = committing.epoch;
        let misplaced = self.misplaced(epoch);
        let flight = self
            .flight
            .as_mut()
            .filter(|flight| flight.epoch == epoch && flight.changed.is_none())
            .ok_or(misplaced)?;
        self.screen
            .resize(flight.grid)
            .map_err(GeometryError::Screen)?;
        flight.changed = Some(receipt.seq);
        self.grid = flight.grid;
        Ok(Resize {
            target: self.screen.target().clone(),
            geometry: (flight.grid.columns(), flight.grid.rows()),
            geometry_epoch: flight.epoch.get(),
        })
    }

    /// The refusal for evidence naming `observed` when it is not the transition in progress.
    pub(super) const fn misplaced(&self, observed: GeometryEpoch) -> GeometryError {
        if observed.get() > self.issued.get() {
            GeometryError::UnknownEpoch {
                observed,
                current: self.issued,
            }
        } else {
            GeometryError::StaleGeometry {
                observed,
                current: self.issued,
            }
        }
    }
}
