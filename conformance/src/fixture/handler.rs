//! The fixture's request handler: the real dispatcher plus methods served directly.
//!
//! Those are the read-only protocol methods `rpc.discover` and paged `terminal.output`,
//! the fixture-only `fixture.transcript` provider write, `fixture.pty` input byte count and
//! `fixture.deliver`, which lets the sequenced terminal's holder stand-in write.

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

use super::agents::Directory;
use super::output;
use super::prompts::Gate;
use super::rotation::Sequenced;
use super::serve::at;
use crate::error::RunnerError;

/// Operation outcomes and agent status facts share the fixture's sequence domain.
pub(crate) type Event = AgentStatusEvent;
pub(crate) type Dispatch = Dispatcher<Event, OperationProducer>;

const DIRECT: [&str; 2] = ["rpc.discover", "terminal.output"];

/// The provider-store stand-in; served but never advertised, since it is no catalog method.
const PROVIDER_WRITE: &str = "fixture.transcript";

/// The PTY stand-in's byte count; served but never advertised.
const PTY_READ: &str = "fixture.pty";

/// Releases the sequenced terminal's write boundary; served but never advertised.
const DELIVER: &str = "fixture.deliver";

#[derive(Debug)]
pub(crate) struct FixtureHandler {
    dispatch: Arc<Dispatch>,
    root: PathBuf,
    pty: (Arc<Gate>, Arc<Directory>),
    sequenced: Arc<Sequenced>,
}

impl FixtureHandler {
    pub(crate) fn new(
        dispatch: Arc<Dispatch>,
        root: PathBuf,
        (gate, directory, sequenced): (Arc<Gate>, Arc<Directory>, Arc<Sequenced>),
    ) -> Self {
        Self {
            dispatch,
            root,
            pty: (gate, directory),
            sequenced,
        }
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
            PTY_READ => super::prompts::pty((&self.pty.0, &self.pty.1), &call.request),
            DELIVER => {
                let _: NoParams = decode_params(&call.request).map_err(BoundaryError::from)?;
                self.sequenced.deliver()
            }
            _ => self.dispatch.call(call).await,
        }
    }
}
