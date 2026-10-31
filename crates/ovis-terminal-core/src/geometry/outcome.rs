//! What a resize request, a completed transition and a lease change resolve to.

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_vt::{ContentRevision, Geometry};

use super::event::commit;
use super::{Dispatch, GeometryEpoch, GeometryError, GeometryEvent};
use crate::events::Commit;
use crate::leases::TerminalLeases;
use crate::registry::TerminalId;

/// The lease table and instant a waiting request is rechecked against before it is sent.
#[derive(Debug, Clone, Copy)]
pub struct Fence<'a> {
    pub(super) leases: &'a TerminalLeases,
    pub(super) at: TimestampMs,
}

impl<'a> Fence<'a> {
    /// Rechecks against `leases` at `at`.
    pub const fn new(leases: &'a TerminalLeases, at: TimestampMs) -> Self {
        Self { leases, at }
    }
}

/// A waiting request a newer one replaced; it never reaches the holder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Superseded {
    /// Operation that was replaced.
    pub op: OpId,
    /// Grid it asked for.
    pub grid: Geometry,
    /// Operation that replaced it.
    pub by: OpId,
}

/// What happened to the request just admitted.
#[derive(Debug)]
#[non_exhaustive]
pub enum Outcome {
    /// The grid already is, or is already becoming, this size; nothing is sent.
    Unchanged {
        /// Grid.
        grid: Geometry,
        /// Epoch that has or will have this grid.
        epoch: GeometryEpoch,
    },
    /// A transition is in progress; this request waits and is sent when it completes.
    Queued {
        /// Grid it waits to apply.
        grid: Geometry,
    },
    /// The transition starts now; commit its change event before anything else.
    Dispatched(Dispatch),
}

/// The admitted request's outcome and the waiting request it replaced, if any.
#[derive(Debug)]
#[non_exhaustive]
pub struct Requested {
    /// Outcome of this request.
    pub outcome: Outcome,
    /// Older waiting request this one superseded.
    pub superseded: Option<Superseded>,
}

/// Final result of one resize operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Resolved {
    /// The grid was applied by the VT and acknowledged by the holder.
    Applied {
        /// Epoch of the change.
        epoch: GeometryEpoch,
        /// Grid applied.
        grid: Geometry,
        /// Content revision of the completing checkpoint.
        revision: ContentRevision,
    },
    /// The grid already had this size when the request's turn came.
    Unchanged {
        /// Grid.
        grid: Geometry,
        /// Epoch that has this grid.
        epoch: GeometryEpoch,
    },
    /// The request's admission no longer holds; nothing was sent.
    Refused(GeometryError),
}

/// One operation and its final result.
#[derive(Debug)]
#[non_exhaustive]
pub struct Resolution {
    /// Operation.
    pub op: OpId,
    /// Result.
    pub resolved: Resolved,
}

/// A completed transition: its events, the operations it resolved and the next transition.
#[derive(Debug)]
#[non_exhaustive]
pub struct Settled {
    /// Terminal.
    pub terminal: TerminalId,
    /// `geometry.checkpoint`, then `geometry.input_ready` unless another transition starts.
    pub events: Vec<GeometryEvent>,
    /// Operations resolved by this completion.
    pub resolutions: Vec<Resolution>,
    /// The waiting request, now dispatched; commit it after `events`.
    pub next: Option<Dispatch>,
}

impl Settled {
    /// One durable unit committing `events` in order under `op`.
    pub fn commit(&self, op: ovis_terminal_store::OpId) -> Option<Commit<GeometryEvent>> {
        commit(self.terminal, self.events.clone(), op)
    }
}
