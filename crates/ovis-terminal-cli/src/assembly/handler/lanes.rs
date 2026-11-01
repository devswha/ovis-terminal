//! Layout reads and edits, selections, changes to a held lease, and the geometry claim
//! an interact attach makes on open, all on the runtime's actor.

use std::sync::Arc;

use ovis_terminal_core::leases::Caller;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::methods::attach::{
    AttachLeaseParams, AttachOpenParams, AttachResizeParams,
};
use ovis_terminal_protocol::schema::methods::layout::{LayoutEditParams, SelectionSetParams};
use ovis_terminal_protocol::schema::records::LeaseRole;
use ovis_terminal_server::conn::Call;
use ovis_terminal_store::OpId as StoreOpId;
use serde_json::{Value, json};

use super::super::actor::now;
use super::super::error::{refusal, worker_gone};
use super::{Served, decode};

impl Served {
    /// After a ready interact attach that named a grid, takes the geometry lease for the
    /// same principal and client (queued behind a holder, never displacing one) and
    /// applies the grid through the geometry controller.
    /// Layout reads and edits, selections, and changes to a held lease, on the actor.
    pub(super) async fn layout_or_lease(&self, call: Call) -> Result<Value, BoundaryError> {
        let session = Arc::clone(&call.session);
        match call.request.method() {
            "layout.get" => {
                let _: NoParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.layout_get(&session))
                    .await
            }
            "layout.edit" => {
                let params: LayoutEditParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.layout_edit(&session, &params))
                    .await
            }
            "selection.set" => {
                let params: SelectionSetParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.select(&session, &params))
                    .await
            }
            "attach.release_geometry" => {
                let params: AttachLeaseParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.release_geometry(&session, &params))
                    .await
            }
            "attach.focus" => {
                let params: AttachLeaseParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.focus(&session, &params))
                    .await
            }
            _ => Err(BoundaryError::method_not_found()),
        }
    }

    pub(super) async fn claim_geometry(
        &self,
        call: (&Arc<ovis_terminal_authz::Session>, &AttachOpenParams),
        opened: &Value,
    ) -> Result<(), BoundaryError> {
        let (session, params) = call;
        let ready = opened.get("outcome").and_then(Value::as_str) == Some("ready");
        let Some(geometry) = params.geometry else {
            return Ok(());
        };
        if !ready || params.role != LeaseRole::Interact {
            return Ok(());
        }
        let resolved = self
            .lane
            .leases(&params.target)
            .map_err(|error| error.boundary())?;
        let held = resolved
            .geometry_owner
            .is_some_and(|owner| owner.holder == params.client_id);
        let lease = if held {
            None
        } else {
            let lane = Arc::clone(&self.lane);
            let (session, client) = (Arc::clone(session), params.client_id.clone());
            let target = params.target.clone();
            let op = StoreOpId::new(format!("{}.geometry", params.op_id.as_str()));
            tokio::task::spawn_blocking(move || {
                let at = now().map_err(|_| refusal("internal", "the clock is unreadable"))?;
                lane.acquire_geometry(Caller::new(&session, &client, at), (&target, op))
                    .map_err(|error| error.boundary())
            })
            .await
            .map_err(|_| worker_gone())??
        };
        let Some(lease) = lease else {
            return Ok(());
        };
        let resize: AttachResizeParams = serde_json::from_value(json!({
            "op_id": format!("{}.geometry", params.op_id.as_str()),
            "target": params.target,
            "lease_id": lease.lease_id,
            "geometry": geometry,
        }))
        .map_err(|_| refusal("invalid_params", "the attach geometry is not a grid"))?;
        let session = Arc::clone(session);
        self.actor
            .call(move |runtime| runtime.resize(&session, &resize))
            .await
            .map(drop)
    }
}
