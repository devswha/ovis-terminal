//! The fixture's request handler: the real dispatcher plus methods served directly.
//!
//! Those are the read-only protocol methods `rpc.discover` and paged `terminal.output`,
//! the fixture-only `fixture.transcript` provider write, `fixture.pty` input byte count,
//! `fixture.bridge` extension driver, the `fixture.exit` and `fixture.program` program
//! lifecycle stand-ins, `fixture.deliver`, which lets the sequenced terminal's holder
//! stand-in write, the `fixture.update` release probe, the `fixture.remote`,
//! `fixture.machine` and `fixture.remote_child` machine stand-ins, `$/orphanedAck`
//! answered by the machine actor lane, and the health stand-ins of [`super::health`].

use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_protocol::domain::machine_actor::ORPHANED_ACK;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::{decode_params, openrpc_document};
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::dispatch::Dispatcher;
use serde_json::Value;

use super::agents::Directory;
use super::bridge::Extension;
use super::machines::{self, Machines};
use super::processes::Programs;
use super::prompts::Gate;
use super::rotation::Sequenced;
use super::serve::at;
use super::tree::{self, Tree};
use super::{health, output};
use crate::error::RunnerError;

/// Operation outcomes and agent status facts share the fixture's sequence domain.
pub(crate) type Event = AgentStatusEvent;
pub(crate) type Dispatch = Dispatcher<Event, OperationProducer>;

const DIRECT: [&str; 2] = ["rpc.discover", "terminal.output"];

/// The provider-store stand-in; served but never advertised, since it is no catalog method.
const PROVIDER_WRITE: &str = "fixture.transcript";

/// The `terminal.kill` plus `terminal.recover` stand-in of the process fixture; never advertised.
const REPLACE_PROCESS: &str = "fixture.replace_process";

/// The PTY stand-in's byte count; served but never advertised.
const PTY_READ: &str = "fixture.pty";

/// Releases the sequenced terminal's write boundary; served but never advertised.
const DELIVER: &str = "fixture.deliver";

/// The in-process extension stand-in's driver; served but never advertised.
const EXTENSION: &str = "fixture.bridge";

/// A fixture program's exit, or the replay of an older one; served but never advertised.
const PROGRAM_EXIT: &str = "fixture.exit";

/// What a fixture terminal's programs received; served but never advertised.
const PROGRAM_READ: &str = "fixture.program";

/// One request through a fixture machine's actor; served but never advertised.
const REMOTE_CALL: &str = "fixture.remote";

/// Freezes, cuts, reconnects, delivers to or inspects a fixture machine; never advertised.
const MACHINE_CONTROL: &str = "fixture.machine";

/// Records that a fixture machine holds a child of a local parent; never advertised.
const REMOTE_CHILD: &str = "fixture.remote_child";

/// Fixture-only stand-ins answered beside the dispatcher.
#[derive(Debug)]
pub(crate) struct Probes {
    pub(crate) gate: Arc<Gate>,
    pub(crate) directory: Arc<Directory>,
    pub(crate) extension: Arc<Extension>,
    pub(crate) sequenced: Arc<Sequenced>,
    pub(crate) tree: Arc<Tree>,
    pub(crate) health: Arc<super::health::Probe>,
    pub(crate) machines: Arc<Machines>,
    pub(crate) programs: Arc<Programs>,
}

#[derive(Debug)]
pub(crate) struct FixtureHandler {
    dispatch: Arc<Dispatch>,
    root: PathBuf,
    probes: Probes,
}

impl FixtureHandler {
    pub(crate) const fn new(dispatch: Arc<Dispatch>, root: PathBuf, probes: Probes) -> Self {
        Self {
            dispatch,
            root,
            probes,
        }
    }

    /// Cancels and joins every dispatcher-owned operation task.
    pub(crate) async fn shutdown(&self) -> Result<(), RunnerError> {
        self.dispatch
            .shutdown()
            .await
            .map_err(at("shut the dispatcher down"))?;
        let programs = Arc::clone(&self.probes.programs);
        tokio::task::spawn_blocking(move || programs.stop())
            .await
            .map_err(at("join the program stop"))?
    }
}

impl FixtureHandler {
    /// Replaces `proc`'s program; a failure is its typed fixture kind, never a fake success.
    async fn replace(&self) -> Result<Value, BoundaryError> {
        let programs = Arc::clone(&self.probes.programs);
        let replaced = tokio::task::spawn_blocking(move || programs.replace()).await;
        let outcome = replaced
            .map_err(at("join the replacement"))
            .and_then(|replaced| replaced);
        outcome.map_err(|error| {
            BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
                .with_hint(error.hint())
        })
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
            super::commit_barrier::METHOD => {
                super::commit_barrier::call(&self.root, &call.request).await
            }
            REPLACE_PROCESS => self.replace().await,
            PTY_READ => {
                let probes = (&*self.probes.gate, &*self.probes.directory);
                super::prompts::pty(probes, &call.request)
            }
            EXTENSION => self.probes.extension.call(&call.request).await,
            health::VIEWER | health::INCIDENT | health::STALE_ENDPOINT => {
                self.probes.health.call(&call.request)
            }
            PROGRAM_EXIT => tree::exit(&self.probes.tree, &call.request).await,
            PROGRAM_READ => tree::program(&self.probes.tree, &call.request),
            DELIVER => {
                let _: NoParams = decode_params(&call.request).map_err(BoundaryError::from)?;
                self.probes.sequenced.deliver()
            }
            super::update::PROBE => super::update::probe(&self.root),
            REMOTE_CALL => machines::call(&self.probes.machines, &call.request).await,
            MACHINE_CONTROL => machines::control(&self.probes.machines, &call.request).await,
            REMOTE_CHILD => machines::remote_child(&self.probes.machines, &call.request).await,
            ORPHANED_ACK => machines::acknowledge(&self.probes.machines, &call.request).await,
            _ => self.dispatch.call(call).await,
        }
    }
}
