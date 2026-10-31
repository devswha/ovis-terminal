//! The fixture's request handler: the real dispatcher plus two read-only protocol methods
//! served directly, `rpc.discover` and paged `terminal.output`.

use std::sync::Arc;

use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
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

pub(crate) type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

const DIRECT: [&str; 2] = ["rpc.discover", "terminal.output"];

#[derive(Debug)]
pub(crate) struct FixtureHandler {
    dispatch: Arc<Dispatch>,
}

impl FixtureHandler {
    pub(crate) const fn new(dispatch: Arc<Dispatch>) -> Self {
        Self { dispatch }
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
            _ => self.dispatch.call(call).await,
        }
    }
}
