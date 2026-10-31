//! A real dispatcher whose `doc.set` appends to one shared document in a single commit unit.

use std::error::Error;
use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{Coordinator, EventError, SnapshotLease};
use ovis_terminal_core::ops::{Completion, OperationError, OperationEvent, OperationProducer};
use ovis_terminal_protocol::schema::methods::doc::DocSetParams;
use ovis_terminal_server::dispatch::{
    Access, Completed, DispatchError, Dispatcher, Execution, Method,
};
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Store, StoreOptions};
use serde_json::{Value, json};

#[path = "socket.rs"]
pub mod socket;
pub(crate) use socket::{Client, at, error_kind, request, runtime};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Core = Coordinator<OperationEvent, OperationProducer>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

#[derive(Debug)]
pub(crate) struct Fixture {
    pub socket: socket::Fixture,
    pub core: Arc<Core>,
    pub dispatch: Arc<Dispatch>,
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
        let mut dispatch =
            Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        dispatch.register(Document {
            machine: identity.machine_id(),
        })?;
        Ok(Self {
            socket,
            core,
            dispatch: Arc::new(dispatch),
        })
    }

    pub(crate) async fn client(&self) -> TestResult<(Client, Value)> {
        let mut client = self.socket.connect().await?;
        let hello = client.hello("1", &self.socket.bearer).await?;
        Ok((client, hello))
    }

    /// Occupies every pinned-snapshot slot the way long-lived page leases would.
    pub(crate) fn pin_all(&self) -> TestResult<Vec<SnapshotLease>> {
        let mut leases = Vec::new();
        loop {
            match self.core.pin_snapshot() {
                Ok(lease) => leases.push(lease),
                Err(EventError::LimitExceeded) => return Ok(leases),
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub(crate) fn release(&self, leases: Vec<SnapshotLease>) -> TestResult {
        for lease in leases {
            self.core.release_snapshot(&lease.token)?;
        }
        Ok(())
    }

    pub(crate) async fn close(self) -> TestResult {
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still owned")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}

/// Appends each write's value under the version the unit read, so a lost update is visible.
#[derive(Debug)]
struct Document {
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
        let key = ResourceKey::new("document", params.name.as_str());
        execution
            .commit_unit(move |snapshot| {
                let (mut writers, version) = match snapshot
                    .resource(&key)
                    .map_err(EventError::from)?
                {
                    ResourceLookup::Live { value, version, .. } => (
                        serde_json::from_slice::<Vec<Value>>(value.as_bytes())?,
                        version,
                    ),
                    ResourceLookup::Absent => (Vec::new(), ovis_terminal_store::Version::ABSENT),
                    _ => return Err(OperationError::InvalidLedger),
                };
                writers.push(params.value);
                let next = version
                    .get()
                    .checked_add(1)
                    .ok_or(OperationError::TooLarge)?;
                let result =
                    json!({"version": next.to_string(), "writers": writers.len().to_string()});
                let write = CasWrite::new(
                    key,
                    version,
                    Some(Payload::from(serde_json::to_vec(&writers)?)),
                );
                Ok(Completion::new(result).write(write))
            })
            .await
    }
}

pub(crate) fn set(id: &str, op: &str, value: &str) -> Value {
    request(
        id,
        "doc.set",
        &json!({"op_id": op, "name": "shared", "value": value, "version": "0"}),
    )
}

pub(crate) fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
