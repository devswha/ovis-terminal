//! What a serving runtime needs from attach state besides streams: the lease table its
//! input and resize admission check against, the screen it paints from, `attach.leases`
//! and the geometry lease an interact client asks for with its first grid.

use ovis_terminal_core::leases::{Acquire, Caller, ControlSlot, LeaseError, TerminalLeases};
use ovis_terminal_core::registry::LeaseId;
use ovis_terminal_core::screen::Screen;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::attach::AttachLeasesResult;
use ovis_terminal_protocol::schema::records::{LeaseRecord, Modes};
use ovis_terminal_protocol::schema::scalar::{EventSeq, Revision};
use ovis_terminal_store::OpId;
use ovis_terminal_vt::{ModeRevision, TerminalModes};

use super::error::StreamError;
use super::hub::TerminalStreams;
use super::lane::AttachLane;
use super::records::{enabled, lease_record};
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

    /// Releases `lease` when it is `target`'s geometry lease and `caller` holds it; the
    /// next waiting interact client, if any, becomes the geometry owner. Answers the owner
    /// afterwards and the committed sequence, `None` when nothing changed.
    ///
    /// # Errors
    ///
    /// `no_lease` unless `lease` is the caller's geometry lease, or a journal failure.
    pub fn release_geometry(
        &self,
        caller: Caller<'_>,
        (target, lease, op): (&Target, LeaseId, OpId),
    ) -> Result<(Option<LeaseRecord>, Option<EventSeq>), StreamError> {
        let hub = self.hub(&self.terminals.resolve(target)?)?;
        let mut state = hub.lock()?;
        let geometry = state.leases.view().geometry.map(|held| held.lease_id);
        if geometry != Some(lease) {
            return Err(LeaseError::NoLease.into());
        }
        let transition = state.leases.release(caller, lease)?;
        let owner = state.leases.view().geometry.as_ref().map(lease_record);
        drop(state);
        let seq = hub.setup.journal.record(op, transition)?;
        Ok((owner, seq))
    }
}

/// The wire record of a VT's modes at `revision`, as checkpoints and screen reads carry it.
#[must_use]
pub fn wire_modes(modes: &TerminalModes, revision: ModeRevision) -> Modes {
    Modes::new(
        enabled(modes),
        modes.keyboard.kitty_flags,
        Revision::new(revision.get()),
    )
}
