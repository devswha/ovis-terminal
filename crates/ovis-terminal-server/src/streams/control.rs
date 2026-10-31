//! What a serving runtime needs from attach state besides streams: the lease table its
//! input and resize admission check against, the screen it paints from, `attach.leases`
//! and the geometry lease an interact client asks for with its first grid.

use ovis_terminal_core::leases::{Acquire, Caller, ControlSlot, LeaseError, TerminalLeases};
use ovis_terminal_core::screen::Screen;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::attach::AttachLeasesResult;
use ovis_terminal_protocol::schema::records::LeaseRecord;
use ovis_terminal_store::OpId;

use super::error::StreamError;
use super::hub::TerminalStreams;
use super::lane::AttachLane;
use super::records::lease_record;
use crate::dispatch::lanes::agent_status::TerminalDirectory;

impl TerminalStreams {
    /// Runs `read` against the terminal's current leases, under the hub lock.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Poisoned`] when the state lock is poisoned.
    pub fn with_leases<R>(
        &self,
        read: impl FnOnce(&TerminalLeases) -> R,
    ) -> Result<R, StreamError> {
        Ok(read(&self.lock()?.leases))
    }

    /// The screen every stream of this terminal paints from.
    #[must_use]
    pub const fn screen(&self) -> &Screen {
        &self.screen
    }
}

impl<T: TerminalDirectory> AttachLane<T> {
    /// `attach.leases`: interact, geometry and observe leases of the terminal `target`
    /// names, as wire records.
    ///
    /// # Errors
    ///
    /// Returns the resolution failure, `not_attachable` or `stale_target`.
    pub fn leases(&self, target: &Target) -> Result<AttachLeasesResult, StreamError> {
        let hub = self.hub(&self.terminals.resolve(target)?)?;
        let view = hub.with_leases(TerminalLeases::view)?;
        let observers: Vec<LeaseRecord> = view.observers.iter().map(lease_record).collect();
        let record = serde_json::json!({
            "interact": view.interact.as_ref().map(lease_record),
            "geometry_owner": view.geometry.as_ref().map(lease_record),
            "observers": observers,
        });
        Ok(serde_json::from_value(record)?)
    }

    /// Requests the geometry lease of `target` for `caller`; a held slot queues the
    /// caller for hand-on instead of displacing anyone. Blocks on the durable commit of
    /// the grant, so call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Returns the resolution, authority or journal failure; `Ok(None)` means queued.
    pub fn acquire_geometry(
        &self,
        caller: Caller<'_>,
        (target, op): (&Target, OpId),
    ) -> Result<Option<LeaseRecord>, StreamError> {
        let hub = self.hub(&self.terminals.resolve(target)?)?;
        let fresh = self.fresh_lease();
        let mut state = hub.lock()?;
        let granted = match state
            .leases
            .acquire(caller, Acquire::new(ControlSlot::Geometry, fresh))
        {
            Ok(granted) => granted,
            Err(LeaseError::Conflict { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        drop(state);
        let record = lease_record(&granted.lease);
        hub.setup.journal.record(op, granted.transition)?;
        Ok(Some(record))
    }
}
