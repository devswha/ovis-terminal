//! The fixture's request handler: the real dispatcher plus two read-only protocol methods
//! served directly, `rpc.discover` and paged `terminal.output`, and the fixture-only
//! `fixture.transcript` provider write.

use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::{decode_params, openrpc_document};
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::dispatch::Dispatcher;
use serde_json::Value;

use super::output;
use super::serve::at;
use crate::error::RunnerError;

/// Operation outcomes and agent status facts share the fixture's sequence domain.
pub(crate) type Event = AgentStatusEvent;
pub(crate) type Dispatch = Dispatcher<Event, OperationProducer>;

const DIRECT: [&str; 2] = ["rpc.discover", "terminal.output"];

/// The provider-store stand-in; served but never advertised, since it is no catalog method.
const PROVIDER_WRITE: &str = "fixture.transcript";

#[derive(Debug)]
pub(crate) struct FixtureHandler {
    dispatch: Arc<Dispatch>,
    root: PathBuf,
}

impl FixtureHandler {
    pub(crate) const fn new(dispatch: Arc<Dispatch>, root: PathBuf) -> Self {
        Self { dispatch, root }
    }

    /// Cancels and joins every dispatcher-owned operation task.
    pub(crate) async fn shutdown(&self) -> Result<(), RunnerError> {
        self.dispatch
            .shutdown()
            .await
            .map_err(at("shut the dispatcher down"))
    }
}

fn discover(request: &RpcRequest) -> Result<Value, BoundaryError> {
    let _: NoParams = decode_params(request).map_err(BoundaryError::from)?;
    openrpc_document().map_err(|_| {
        BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
            .with_hint("rebuild the schema document")
    })
}

impl RequestHandler for FixtureHandler {
    fn methods(&self) -> Vec<&'static str> {
        let mut methods = self.dispatch.methods();
        methods.extend(DIRECT);
        methods
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        match call.request.method() {
            "rpc.discover" => discover(&call.request),
            "terminal.output" => output::page(&call.request),
            PROVIDER_WRITE => super::transcripts::write(&self.root, &call.request),
            _ => self.dispatch.call(call).await,
        }
    }
}
