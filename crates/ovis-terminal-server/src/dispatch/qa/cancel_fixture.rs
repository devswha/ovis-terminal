//! A real dispatcher whose `doc.set` records handler entry and its file effect.

use std::collections::BTreeSet;
use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_protocol::schema::methods::doc::DocSetParams;
use ovis_terminal_server::dispatch::{
    Access, Completed, DispatchError, Dispatcher, Execution, Method,
};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::sync::Notify;

#[path = "socket.rs"]
pub mod socket;
pub(crate) use socket::{Client, at, error_kind, request, runtime};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Core = Coordinator<OperationEvent, OperationProducer>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

/// Names whose handler ran, and the release that lets a held handler commit.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    pub entered: Mutex<BTreeSet<String>>,
    pub release: Notify,
}

impl Gate {
    pub(crate) fn entered(&self, name: &str) -> TestResult<bool> {
        Ok(self
            .entered
            .lock()
            .map_err(|_| "gate poisoned")?
            .contains(name))
    }
}

#[derive(Debug)]
pub(crate) struct Fixture {
    pub socket: socket::Fixture,
    core: Arc<Core>,
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
        std::fs::create_dir(socket.path("effects"))?;
        let mut dispatch =
            Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        dispatch.register(Document {
            gate: Arc::clone(&gate),
            effects: socket.path("effects"),
            machine: identity.machine_id(),
        })?;
        Ok(Self {
            socket,
            core,
            dispatch: Arc::new(dispatch),
            gate,
        })
    }

    pub(crate) fn effect(&self, name: &str) -> PathBuf {
        self.socket.path("effects").join(name)
    }

    pub(crate) async fn client(&self, bearer: &str) -> TestResult<Client> {
        let mut client = self.socket.connect().await?;
        client.hello("1", bearer).await?;
        Ok(client)
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
struct Document {
    gate: Arc<Gate>,
    effects: PathBuf,
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
            .lock()
            .map_err(|_| std::io::Error::other("fixture gate poisoned"))?
            .insert(params.name.as_str().to_owned());
        tokio::select! {
            () = released => {},
            () = execution.cancelled() => {},
        }
        let path = self.effects.join(params.name.as_str());
        execution
            .commit(move || {
                std::fs::write(path, serde_json::to_vec(&params.value)?)?;
                Ok(json!({"name": params.name, "value": params.value}))
            })
            .await
    }
}

pub(crate) fn set(id: &str, op: &str, (name, value): (&str, &str)) -> Value {
    request(
        id,
        "doc.set",
        &json!({"op_id": op, "name": name, "value": value, "version": "0"}),
    )
}

pub(crate) fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
