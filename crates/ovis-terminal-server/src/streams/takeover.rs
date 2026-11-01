//! `attach.takeover`: an explicit, authorized takeover of the interact lease.
//!
//! The lease moves onto a stream the caller already holds. The displaced holder's
//! interact streams close `taken_over` naming the new owner; only once the transition is
//! durable is the caller's stream made interact and told input is ready. A journal
//! failure releases the caller's leases and closes its stream `lease_revoked`, so no
//! lease outlives an uncommitted takeover.

use ovis_terminal_authz::Session;
use ovis_terminal_core::leases::{Caller, ControlSlot, Owner, Takeover};
use ovis_terminal_core::registry::LeaseId;
use ovis_terminal_protocol::schema::events::StreamCloseReason;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::methods::attach::{AttachTakeoverResult, TakeoverRequest};
use ovis_terminal_protocol::schema::records::{AttachControl, ControlKind, StreamClose};
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};
use ovis_terminal_store::OpId;
use serde_json::json;

use super::error::StreamError;
use super::hub::{HubState, Phase, TerminalStreams};
use super::open::displace;
use super::records::lease_record;

/// One authenticated takeover onto `stream`, with the lease identity minted for it.
#[derive(Debug, Clone, Copy)]
pub(super) struct TakeoverCall<'a> {
    pub session: &'a Session,
    pub stream: &'a StreamId,
    pub takeover: &'a TakeoverRequest,
    pub fresh: LeaseId,
    pub at: TimestampMs,
}

/// Releases the leases of a takeover the journal refused and closes its stream.
fn abandon(state: &mut HubState, (stream, owner): (&StreamId, &Owner)) -> Result<(), StreamError> {
    state.leases.disconnect(owner)?;
    if let Some(viewer) = state.viewers.get_mut(stream) {
        let close = StreamClose::new(StreamCloseReason::LeaseRevoked, viewer.next_offset().wire());
        viewer.close_now(close);
        viewer.wake.notify_one();
    }
    Ok(())
}

/// Makes the caller's stream interact and tells it input is ready.
fn ready(state: &mut HubState, stream: &StreamId) {
    if let Some(viewer) = state.viewers.get_mut(stream) {
        viewer.interact = true;
        let at = viewer.next_offset().wire();
        viewer.control(AttachControl::new(
            stream.clone(),
            ControlKind::InputReady,
            at,
        ));
        viewer.wake.notify_one();
    }
}

impl TerminalStreams {
    /// Moves the interact lease to the owner of `request.stream` under `op`.
    pub(super) fn take_over(
        &self,
        request: TakeoverCall<'_>,
        op: OpId,
    ) -> Result<AttachTakeoverResult, StreamError> {
        let mut state = self.lock()?;
        if !matches!(state.phase, Phase::Live) {
            return Err(StreamError::NotAttachable);
        }
        let owner = state
            .viewers
            .get(request.stream)
            .map(|viewer| viewer.owner.clone())
            .ok_or(StreamError::StreamNotFound)?;
        let caller = Caller::new(request.session, &owner.client, request.at);
        let takeover = Takeover::new(ControlSlot::Interact, request.takeover, request.fresh)?;
        let granted = state.leases.take_over(caller, &takeover)?;
        let displaced = displace(&mut state, &granted);
        drop(state);
        let acquired = lease_record(&granted.lease);
        let recorded = self.setup.journal.record(op, granted.transition);
        let mut state = self.lock()?;
        let seq: EventSeq = match recorded {
            Ok(Some(seq)) => seq,
            Ok(None) => {
                abandon(&mut state, (request.stream, &owner))?;
                return Err(StreamError::NotAttachable);
            }
            Err(error) => {
                abandon(&mut state, (request.stream, &owner))?;
                return Err(error);
            }
        };
        ready(&mut state, request.stream);
        drop(state);
        Ok(serde_json::from_value(
            json!({"acquired": acquired, "displaced": displaced, "seq": seq}),
        )?)
    }
}
