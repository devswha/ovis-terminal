//! The served method table. Every name here is routed to a landed implementation and hello
//! advertises exactly these plus the attach stream methods; anything else is
//! `method_not_found`, never a stub result.

use std::sync::Arc;

use ovis_terminal_core::leases::Caller;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::limits::TransportKind;
use ovis_terminal_protocol::schema::methods::attach::{
    AttachLeaseParams, AttachLeasesParams, AttachOpenParams, AttachResizeParams,
};
use ovis_terminal_protocol::schema::methods::input::InputBarrierParams;
use ovis_terminal_protocol::schema::methods::layout::{LayoutEditParams, SelectionSetParams};
use ovis_terminal_protocol::schema::methods::server::OpRefParams;
use ovis_terminal_protocol::schema::methods::terminal::{TerminalGetParams, TerminalRunParams};
use ovis_terminal_protocol::schema::records::LeaseRole;
use ovis_terminal_server::authn::Revalidated;
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::dispatch::lanes::boundary_policy::BoundaryPolicy;
use ovis_terminal_server::dispatch::lanes::health::Measured;
use ovis_terminal_server::dispatch::lanes::prompt_bridge::Gated;
use ovis_terminal_server::streams::{AttachHandler, AttachLane};
use ovis_terminal_store::OpId as StoreOpId;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::actor::{Actor, now};
use super::directory::Directory;
use super::error::{refusal, worker_gone};
use super::lanes::Dispatch;

/// Methods served by the runtime actor and the lease table, besides the attach streams.
pub(super) const ROUTED: [&str; 32] = [
    "terminal.run",
    "terminal.list",
    "terminal.get",
    "terminal.kill",
    "terminal.rm",
    "terminal.keep",
    "terminal.rename",
    "terminal.label",
    "terminal.recover",
    "terminal.process_info",
    "terminal.screen",
    "terminal.output",
    "terminal.wait_output",
    "op.get",
    "input.raw",
    "input.send",
    "input.keys",
    "input.paste",
    "input.barrier",
    "attach.resize",
    "attach.leases",
    "attach.focus",
    "attach.release_geometry",
    "layout.get",
    "layout.edit",
    "selection.set",
    "turn.wait",
    "events.wait",
    "input.interrupt",
    "events.subscribe",
    "events.update",
    "events.unsubscribe",
];

type Lane = AttachLane<Arc<Directory>>;

pub(super) fn decode<P: DeserializeOwned>(call: &Call) -> Result<P, BoundaryError> {
    decode_params(&call.request).map_err(BoundaryError::from)
}

/// Lifecycle, input, resize and lease reads.
#[derive(Debug)]
pub(super) struct Served {
    pub(super) actor: Actor,
    pub(super) lane: Arc<Lane>,
    pub(super) dispatch: Arc<Dispatch>,
    pub(super) events: Arc<super::events::EventLane>,
    pub(super) stop: super::restart::StopSignal,
    pub(super) restart: super::restart::RestartSignal,
}

impl RequestHandler for Served {
    fn methods(&self) -> Vec<&'static str> {
        let mut methods = ROUTED.to_vec();
        methods.extend(
            self.dispatch
                .methods()
                .into_iter()
                .filter(|method| !ROUTED.contains(method)),
        );
        methods
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        let session = Arc::clone(&call.session);
        match call.request.method() {
            "terminal.run" => {
                let params: TerminalRunParams = decode(&call)?;
                let run = move |runtime: &mut super::actor::Runtime| {
                    runtime.run_terminal(&session, params)
                };
                self.actor.call(run).await
            }
            "terminal.list" => {
                let _: ovis_terminal_protocol::schema::methods::terminal::TerminalListParams =
                    decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.list_terminals(&session))
                    .await
            }
            "terminal.get" => {
                let params: TerminalGetParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.get_terminal(&session, &params.target))
                    .await
            }
            "terminal.kill"
            | "terminal.rm"
            | "terminal.keep"
            | "terminal.rename"
            | "terminal.label"
            | "terminal.recover"
            | "terminal.process_info" => self.mutate(call).await,
            "terminal.screen" | "terminal.output" | "terminal.wait_output" => self.read(call).await,
            "op.get" => {
                let params: OpRefParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.operation(&session, &params.op_id))
                    .await
            }
            "input.raw" | "input.send" | "input.keys" | "input.paste" => self.write(call).await,
            "attach.resize" => {
                let params: AttachResizeParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.resize(&session, &params))
                    .await
            }
            "layout.get"
            | "layout.edit"
            | "selection.set"
            | "attach.release_geometry"
            | "attach.focus" => self.layout_or_lease(call).await,
            "turn.wait" | "events.wait" | "input.interrupt" => self.waited(call).await,
            "events.subscribe" | "events.update" | "events.unsubscribe" => self.events(call).await,
            "attach.leases" => {
                let params: AttachLeasesParams = decode(&call)?;
                let leases = self
                    .lane
                    .leases(&params.target)
                    .map_err(|error| error.boundary())?;
                serde_json::to_value(leases).map_err(|_| refusal("internal", "report the build"))
            }
            "server.stop" => {
                let stopped = self.dispatch.call(call).await?;
                self.stop.stopped();
                Ok(stopped)
            }
            "server.restart" | "update.activate" | "update.rollback" => {
                let op = call.request.params().get("op_id").and_then(Value::as_str);
                let op = op.map(str::to_owned);
                let restarted = self.dispatch.call(call).await;
                if let Some(op) = op {
                    self.restart.requested(op);
                }
                restarted
            }
            "input.barrier" => {
                let params: InputBarrierParams = decode(&call)?;
                self.actor
                    .call(move |runtime| runtime.barrier(&params))
                    .await
            }
            _ => self.dispatch.call(call).await,
        }
    }
}

impl Served {
    /// After a ready interact attach that named a grid, takes the geometry lease for the
    /// same principal and client (queued behind a holder, never displacing one) and
    /// applies the grid through the geometry controller.
    /// Layout reads and edits, selections, and changes to a held lease, on the actor.
    async fn layout_or_lease(&self, call: Call) -> Result<Value, BoundaryError> {
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

    async fn claim_geometry(
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

/// The connection handler: credentials are revalidated first, then every call's latency is
/// measured around the boundary policy, which audits it and enforces payload and path
/// bounds; the prompt bridge gate then keeps a program token to its own terminal before
/// the routes see the call.
pub(super) type Handler =
    Revalidated<Measured<BoundaryPolicy<Gated<Assembled, Arc<super::directory::Directory>>>>>;

/// The attach stream methods in front of [`Served`], with the geometry claim on open.
#[derive(Debug)]
pub(super) struct Assembled {
    pub(super) attach: AttachHandler<Served, Arc<Directory>>,
    pub(super) served: Arc<Served>,
    pub(super) transports: Vec<TransportKind>,
}

impl RequestHandler for Assembled {
    fn methods(&self) -> Vec<&'static str> {
        let mut methods = self.attach.methods();
        methods.push(super::discovery::HELLO);
        methods.extend(super::discovery::DISCOVERY);
        methods
    }

    fn transports(&self) -> Vec<TransportKind> {
        self.transports.clone()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        if super::discovery::DISCOVERY.contains(&call.request.method()) {
            return super::discovery::answer(&call, &self.transports);
        }
        if call.request.method() != "attach.open" {
            return self.attach.call(call).await;
        }
        let params: AttachOpenParams = decode(&call)?;
        let session = Arc::clone(&call.session);
        let opened = self.attach.call(call).await?;
        self.served
            .claim_geometry((&session, &params), &opened)
            .await?;
        Ok(opened)
    }
}
