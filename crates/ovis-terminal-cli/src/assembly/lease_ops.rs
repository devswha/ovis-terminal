//! `attach.release_geometry` and `attach.focus` on a live terminal's lease table and
//! geometry controller, under the caller's own lease.
//!
//! The daemon applies a resize synchronously inside the actor (commit, VT, holder,
//! settle), so the controller never holds a queued request a lease change could fence.

use ovis_terminal_authz::Session;
use ovis_terminal_core::leases::{Caller, TerminalLeases};
use ovis_terminal_core::registry::LeaseId;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::attach::{AttachLeaseParams, AttachLeaseResult};
use ovis_terminal_protocol::schema::records::LeaseRecord;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::streams::lease_record;
use ovis_terminal_store::OpId as StoreOpId;
use serde_json::{Value, json};

use super::actor::{Runtime, now};
use super::error::refusal;
use super::input::{live_terminal, presented_client};

fn internal<E: std::fmt::Debug>(cause: E) -> BoundaryError {
    super::error::internal("attach lease change")(cause)
}

fn lease_of(params: &AttachLeaseParams) -> Result<LeaseId, BoundaryError> {
    params
        .lease_id
        .as_str()
        .parse()
        .map_err(|_| refusal("invalid_params", "lease_id must name a lease"))
}

fn answer(geometry_owner: Option<&LeaseRecord>, seq: EventSeq) -> Result<Value, BoundaryError> {
    let value = json!({"geometry_owner": geometry_owner, "seq": seq});
    let checked: AttachLeaseResult = serde_json::from_value(value).map_err(internal)?;
    serde_json::to_value(checked).map_err(internal)
}

impl Runtime {
    /// Releases the caller's geometry lease; the next waiting interact client, if any,
    /// becomes the geometry owner. Only the current geometry lease can be released here.
    pub(super) fn release_geometry(
        &mut self,
        session: &Session,
        params: &AttachLeaseParams,
    ) -> Result<Value, BoundaryError> {
        let lease = lease_of(params)?;
        let op = StoreOpId::new(params.op_id.as_str());
        let Self {
            directory,
            live: served,
            lane,
            journal,
            ..
        } = self;
        let live = live_terminal((directory, served), &params.target)?;
        let view = live
            .hub
            .with_leases(TerminalLeases::view)
            .map_err(internal)?;
        let client = presented_client(&view, (session, lease));
        let caller = Caller::new(session, &client, now().map_err(internal)?);
        let (owner, recorded) = lane
            .release_geometry(caller, (&params.target, lease, op))
            .map_err(|error| error.boundary())?;
        let seq = match recorded {
            Some(seq) => seq,
            None => journal.cursor().map_err(|error| error.boundary())?,
        };
        answer(owner.as_ref(), seq)
    }

    /// Records that the caller's client, holding `lease`, has focus; focus never grants
    /// input. `seq` is the lease log position the reported geometry owner reflects.
    pub(super) fn focus(
        &mut self,
        session: &Session,
        params: &AttachLeaseParams,
    ) -> Result<Value, BoundaryError> {
        let lease = lease_of(params)?;
        let Self {
            directory,
            live: served,
            journal,
            ..
        } = self;
        let live = live_terminal((directory, served), &params.target)?;
        let view = live
            .hub
            .with_leases(TerminalLeases::view)
            .map_err(internal)?;
        let client = presented_client(&view, (session, lease));
        let focused = live
            .geometry
            .focus(&view, (session, &client, lease))
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let seq = journal.cursor().map_err(|error| error.boundary())?;
        answer(
            focused.geometry_owner.as_ref().map(lease_record).as_ref(),
            seq,
        )
    }
}
