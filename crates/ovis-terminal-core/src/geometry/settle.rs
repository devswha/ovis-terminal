//! Completing a transition on the holder's acknowledgement, and fencing waiting requests.

use super::controller::{GeometryController, Next, Queued};
use super::{
    Dispatch, Fence, GeometryEpoch, GeometryError, GeometryEvent, Resolution, Resolved, Settled,
};

/// What became of the waiting request when the transition before it completed.
enum Resumed {
    Resolved(Resolution),
    Dispatched(Dispatch),
}

impl GeometryController {
    /// The holder acknowledged delivery of `epoch`; with the VT already at that grid the
    /// runtime records the checkpoint and re-asserts input readiness, or starts the
    /// waiting request after rechecking its admission against `fence`.
    ///
    /// # Errors
    ///
    /// Returns `stale_geometry` for an epoch that is not in progress, `invalid_params`
    /// for one never issued, `not_ready` before the VT applied it, or the screen's refusal.
    pub fn acknowledged(
        &mut self,
        epoch: GeometryEpoch,
        fence: Fence<'_>,
    ) -> Result<Settled, GeometryError> {
        let changed = match &self.flight {
            Some(flight) if flight.epoch == epoch => flight
                .changed
                .ok_or(GeometryError::NotReady { pending: epoch })?,
            Some(_) | None => return Err(self.misplaced(epoch)),
        };
        let paint = self.screen.first_paint().map_err(GeometryError::Screen)?;
        let flight = self
            .flight
            .take()
            .ok_or(GeometryError::NotReady { pending: epoch })?;
        let (grid, revision) = (paint.geometry, paint.revision);
        let mut events = vec![GeometryEvent::Checkpoint {
            terminal: self.terminal,
            epoch,
            grid,
            offset: paint.offset,
            revision,
            changed_seq: changed,
        }];
        let mut resolutions = vec![Resolution {
            op: flight.op,
            resolved: Resolved::Applied {
                epoch,
                grid,
                revision,
            },
        }];
        let next = match self.queued.take().map(|queued| self.resume(queued, fence)) {
            Some(Ok(Resumed::Dispatched(dispatch))) => Some(dispatch),
            Some(Ok(Resumed::Resolved(resolution))) => {
                resolutions.push(resolution);
                None
            }
            Some(Err(error)) => return Err(error),
            None => None,
        };
        if next.is_none() {
            events.push(GeometryEvent::InputReady {
                terminal: self.terminal,
                epoch,
                grid,
                revision,
            });
        }
        Ok(Settled {
            terminal: self.terminal,
            events,
            resolutions,
            next,
        })
    }

    fn resume(&mut self, queued: Queued, fence: Fence<'_>) -> Result<Resumed, GeometryError> {
        if let Err(error) = fence.leases.recheck(&queued.admitted, fence.at) {
            return Ok(Resumed::Resolved(Resolution {
                op: queued.op,
                resolved: Resolved::Refused(GeometryError::Lease(error)),
            }));
        }
        if queued.grid == self.grid {
            return Ok(Resumed::Resolved(Resolution {
                op: queued.op,
                resolved: Resolved::Unchanged {
                    grid: self.grid,
                    epoch: self.issued,
                },
            }));
        }
        let next = Next {
            op: queued.op,
            grid: queued.grid,
            owner: queued.owner,
        };
        self.dispatch(next).map(Resumed::Dispatched)
    }

    /// After a lease change, refuses the waiting request whose admission no longer holds,
    /// so a released or taken-over owner's resize is never sent.
    pub fn lease_changed(&mut self, fence: Fence<'_>) -> Option<Resolution> {
        let queued = self.queued.take()?;
        match fence.leases.recheck(&queued.admitted, fence.at) {
            Ok(()) => {
                self.queued = Some(queued);
                None
            }
            Err(error) => Some(Resolution {
                op: queued.op,
                resolved: Resolved::Refused(GeometryError::Lease(error)),
            }),
        }
    }
}
