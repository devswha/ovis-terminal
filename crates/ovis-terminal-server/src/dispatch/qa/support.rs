//! Real store, authority and transport for dispatch contract tests.

use std::error::Error;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_protocol::schema::methods::doc::DocSetParams;
use ovis_terminal_server::dispatch::{
    Access, Completed, DispatchError, Dispatcher, Execution, Method,
};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

#[path = "socket.rs"]
pub mod socket;
pub(crate) use socket::{at, error_kind, request, runtime};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;
pub(crate) type Core = Coordinator<OperationEvent, OperationProducer>;
pub(crate) type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

#[derive(Debug)]
pub(crate) struct Fixture {
    pub socket: socket::Fixture,
    pub core: Arc<Core>,
    pub dispatch: Arc<Dispatch>,
    pub gate: Arc<Gate>,
}

impl Fixture {
    pub(crate) fn new() -> TestResult<Self> {
        let socket = socket::Fixture::new()?;
        let identity = runtime(1)?;
        let store = Store::open(&socket.path("db"), StoreOptions::default())?;
        let core = Arc::new(Coordinator::new(
            store,
            identity,
            OperationProducer::new(identity.runtime_id()),
        ));
        let gate = Arc::new(Gate::default());
        let mut dispatch =
            Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        dispatch.register(Document {
            gate: Arc::clone(&gate),
            effect: socket.path("effect"),
            machine: identity.machine_id(),
        })?;
        dispatch.register(super::targeting::Terminal::new(socket.path("effects"))?)?;
        Ok(Self {
            socket,
            core,
            dispatch: Arc::new(dispatch),
            gate,
        })
    }

    pub(crate) async fn close(self) -> TestResult {
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still owned")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Gate {
    pub entered: watch::Sender<usize>,
    pub release: Notify,
    pub effect_started: watch::Sender<bool>,
    pub effect_released: Mutex<bool>,
    pub effect_signal: Condvar,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            entered: watch::channel(0).0,
            release: Notify::new(),
            effect_started: watch::channel(false).0,
            effect_released: Mutex::new(true),
            effect_signal: Condvar::new(),
        }
    }
}

#[derive(Debug)]
struct Document {
    gate: Arc<Gate>,
    effect: PathBuf,
    machine: ovis_terminal_core::registry::MachineId,
}

impl Method<OperationEvent, OperationProducer> for Document {
    const NAME: &'static str = "doc.set";
    type Params = DocSetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::DocumentWrite,
            ResolvedTarget::new(
                self.machine,
                TargetResource::Document {
                    name: params.name.clone(),
                },
            ),
        ))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<OperationEvent, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let released = self.gate.release.notified();
        self.gate
            .entered
            .send_modify(|count| *count = count.saturating_add(1));
        tokio::select! {
            () = released => {},
            () = execution.cancelled() => {},
        }
        // A real filesystem effect at dispatch's boundary; not a document-service claim.
        let effect = self.effect.clone();
        let gate = Arc::clone(&self.gate);
        execution
            .commit(move || {
                gate.effect_started.send_replace(true);
                let mut released = gate
                    .effect_released
                    .lock()
                    .map_err(|_| std::io::Error::other("fixture gate poisoned"))?;
                while !*released {
                    released = gate
                        .effect_signal
                        .wait(released)
                        .map_err(|_| std::io::Error::other("fixture gate poisoned"))?;
                }
                drop(released);
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(effect)?;
                file.write_all(serde_json::to_string(&params.value)?.as_bytes())?;
                Ok(json!({"version": params.version, "value": params.value}))
            })
            .await
    }
}

pub(crate) fn params(op: &str) -> Value {
    json!({"op_id": op, "name": "test", "value": "payload", "version": "0"})
}

pub(crate) fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
