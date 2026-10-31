//! `attach.resize` through the landed geometry controller: admission only under the
//! geometry lease, `geometry.changed` committed before the holder hears of the grid, the
//! VT applied first, then the holder, then the checkpoint and input readiness committed.

use ovis_terminal_authz::Session;
use ovis_terminal_core::geometry::{
    Committed, Dispatch, Fence, GeometryAuthorization, Outcome, Resolved, Settled,
};
use ovis_terminal_core::leases::{Caller, TerminalLeases};
use ovis_terminal_core::registry::LeaseId;
use ovis_terminal_holder::unix::Holder;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::attach::{AttachResizeParams, AttachResizeResult};
use ovis_terminal_store::OpId as StoreOpId;
use serde_json::{Value, json};

use super::actor::{Runtime, now};
use super::boot::GeometryLog;
use super::error::refusal;
use super::input::{live_terminal, owned_client};
use super::live::Live;

fn internal<E: std::fmt::Debug>(cause: E) -> BoundaryError {
    super::error::internal("geometry transition")(cause)
}

fn result(outcome: &str, (cols, rows): (u16, u16), revision: u64) -> Result<Value, BoundaryError> {
    let value = json!({
        "outcome": outcome,
        "geometry": {"cols": cols, "rows": rows},
        "revision": revision.to_string(),
    });
    let checked: AttachResizeResult = serde_json::from_value(value).map_err(internal)?;
    serde_json::to_value(checked).map_err(internal)
}

/// Commits the change, applies it to the VT and the holder, then settles it.
fn transition(
    live: &mut Live,
    (dispatch, op): (Dispatch, StoreOpId),
    (log, holder): (&GeometryLog, &Holder),
) -> Result<Settled, BoundaryError> {
    let epoch = dispatch.epoch();
    let committed: Committed = dispatch
        .commit(op, |commit| log.commit(commit))
        .map_err(internal)?;
    let resize = live
        .geometry
        .apply(&committed)
        .map_err(|error| refusal(error.kind(), error.hint()))?;
    holder
        .resize(&resize)
        .map_err(|error| refusal(error.kind(), error.hint()))?;
    let at = now().map_err(internal)?;
    let geometry = &mut live.geometry;
    let settled = live
        .hub
        .with_leases(|leases| geometry.acknowledged(epoch, Fence::new(leases, at)))
        .map_err(|error| error.boundary())?
        .map_err(|error| refusal(error.kind(), error.hint()))?;
    if let Some(commit) = settled.commit(StoreOpId::new(uuid::Uuid::now_v7().to_string())) {
        log.commit(commit).map_err(internal)?;
    }
    Ok(settled)
}

impl Runtime {
    /// Resizes under the caller's geometry lease; the grid is reported as committed.
    pub(super) fn resize(
        &mut self,
        session: &Session,
        params: &AttachResizeParams,
    ) -> Result<Value, BoundaryError> {
        let op = StoreOpId::new(params.op_id.as_str());
        let lease: LeaseId = params
            .lease_id
            .as_str()
            .parse()
            .map_err(|_| refusal("invalid_params", "lease_id must name a lease"))?;
        let Self {
            directory,
            live: served,
            geometry_log,
            holder,
            ..
        } = self;
        let live = live_terminal((directory, served), &params.target)?;
        let view = live
            .hub
            .with_leases(TerminalLeases::view)
            .map_err(internal)?;
        let client = owned_client(&view, (session, lease))?;
        let caller = Caller::new(session, &client, now().map_err(internal)?);
        let admitted = live
            .hub
            .with_leases(|leases| GeometryAuthorization::admit(leases, caller, lease))
            .map_err(|error| error.boundary())?
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let requested = live
            .geometry
            .request(admitted, params)
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let grid = (params.geometry.cols, params.geometry.rows);
        let dispatch = match requested.outcome {
            Outcome::Dispatched(dispatch) => dispatch,
            Outcome::Unchanged { .. } => {
                let revision = live.hub.screen().live().map_err(internal)?.revision.get();
                return result("unchanged", grid, revision);
            }
            _ => {
                return Err(refusal(
                    "not_ready",
                    "retry once the current resize settles",
                ));
            }
        };
        let settled = transition(live, (dispatch, op), (geometry_log, holder))?;
        let resolution = settled
            .resolutions
            .into_iter()
            .find(|resolution| resolution.op.as_str() == params.op_id.as_str());
        match resolution {
            Some(resolution) => match resolution.resolved {
                Resolved::Applied { revision, .. } => result("applied", grid, revision.get()),
                Resolved::Unchanged { .. } => result("unchanged", grid, 0),
                Resolved::Refused(error) => Err(refusal(error.kind(), error.hint())),
                _ => Err(internal("unknown resolution")),
            },
            None => Err(internal("the transition did not resolve this operation")),
        }
    }
}
