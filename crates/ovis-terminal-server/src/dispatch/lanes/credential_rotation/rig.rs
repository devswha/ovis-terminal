//! The real dispatcher and credential lane behind the rotation tests' socket.

use std::sync::{Arc, Mutex};

use ovis_terminal_authz::{
    Action, Grant, Issuance, IssueRequest, IssuedCredential, Permission, PrincipalId,
    ResolvedTarget, ResourceScope, ScopeResource, Session, TargetResource,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::domain::credential_rotation::{CommitRace, FenceToken, RacedWrite};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::doc::DocSetParams;
use ovis_terminal_protocol::schema::scalar::{ByteCount, EventSeq, TimestampMs};
use ovis_terminal_server::authn::Revalidated;
use ovis_terminal_server::dispatch::lanes::credential_rotation::{
    CredentialLane, QueuedWrites, RevokeCredential, RotateCredential,
};
use ovis_terminal_server::dispatch::{
    Access, Completed, DispatchError, Dispatcher, Execution, Method,
};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use crate::support::socket::{self, Client};
use crate::support::{TestResult, request, runtime};

type Core = Coordinator<OperationEvent, OperationProducer>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

/// Stands in for the runtime's terminals.
///
/// It reports one queued write per recheck it is asked for, so a test can see that each
/// withdrawal rechecks queued writes exactly once.
#[derive(Debug, Default)]
pub(crate) struct Terminals(pub(crate) Mutex<u64>);

impl QueuedWrites for Terminals {
    fn recheck(&self) -> Vec<RacedWrite> {
        let Ok(mut asked) = self.0.lock() else {
            return Vec::new();
        };
        *asked = asked.saturating_add(1);
        vec![RacedWrite::new(
            (OpId::new(format!("queued-{asked}")), EventSeq::new(*asked)),
            ByteCount::new(2),
            CommitRace::CancelledBeforeCommit {
                reason: FenceToken::new("credential_revoked".to_owned()),
            },
        )]
    }
}

/// A document write parked before its commit until released.
#[derive(Debug)]
pub(crate) struct Parked {
    pub(crate) entered: watch::Sender<bool>,
    pub(crate) release: Notify,
    pub(crate) committed: Mutex<bool>,
    machine: ovis_terminal_core::registry::MachineId,
}

#[derive(Debug)]
struct ParkedDoc(Arc<Parked>);

impl Method<OperationEvent, OperationProducer> for ParkedDoc {
    const NAME: &'static str = "doc.set";
    type Params = DocSetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let name = params.name.clone();
        Ok(Access::new(
            Action::DocumentWrite,
            ResolvedTarget::new(self.0.machine, TargetResource::Document { name }),
        ))
    }

    async fn run(
        &self,
        _: Self::Params,
        execution: Execution<OperationEvent, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let released = self.0.release.notified();
        self.0.entered.send_replace(true);
        released.await;
        let parked = Arc::clone(&self.0);
        execution
            .commit(move || {
                let mut committed = parked
                    .committed
                    .lock()
                    .map_err(|_| std::io::Error::other("poisoned"))?;
                *committed = true;
                drop(committed);
                Ok(json!({"written": true}))
            })
            .await
    }
}

pub(crate) struct Rig {
    pub(crate) socket: socket::Fixture,
    core: Arc<Core>,
    pub(crate) handler: Arc<Revalidated<Dispatch>>,
    pub(crate) parked: Arc<Parked>,
    pub(crate) terminals: Arc<Terminals>,
}

impl Rig {
    pub(crate) fn new() -> TestResult<Self> {
        let socket = socket::Fixture::new()?;
        let identity = runtime(1)?;
        let store = Store::open(&socket.path("db"), StoreOptions::default())?;
        let core = Arc::new(Coordinator::new(
            store,
            identity,
            OperationProducer::new(identity.runtime_id()),
        ));
        let terminals = Arc::new(Terminals::default());
        let writes: Arc<dyn QueuedWrites> = Arc::clone(&terminals) as Arc<dyn QueuedWrites>;
        let lane =
            CredentialLane::new(Arc::clone(&socket.authority), identity.machine_id(), writes);
        let parked = Arc::new(Parked {
            entered: watch::channel(false).0,
            release: Notify::new(),
            committed: Mutex::new(false),
            machine: identity.machine_id(),
        });
        let mut dispatch =
            Dispatch::new(Arc::clone(&socket.authority), identity, Arc::clone(&core));
        dispatch.register(RotateCredential::new(&lane))?;
        dispatch.register(RevokeCredential::new(&lane))?;
        dispatch.register(ParkedDoc(Arc::clone(&parked)))?;
        let handler = Arc::new(Revalidated::new(Arc::clone(&socket.authority), dispatch));
        Ok(Self {
            socket,
            core,
            handler,
            parked,
            terminals,
        })
    }

    pub(crate) fn writer(&self) -> TestResult<IssuedCredential<RegistryIdentities>> {
        let scope = ResourceScope::new(runtime(1)?.machine_id(), ScopeResource::Machine);
        let grants = [
            Permission::Observe,
            Permission::Interact,
            Permission::PromptAnswer,
        ]
        .into_iter()
        .map(|permission| Grant::new(permission, scope.clone()))
        .collect();
        let request = IssueRequest::new(
            PrincipalId::parse("writer")?,
            grants,
            TimestampMs::new(u64::MAX),
        );
        let owner = self.socket.authority.authenticate(
            &ovis_terminal_authz::EndpointPolicy::new(ovis_terminal_authz::LocalAccount::Unix(
                ovis_terminal_authz::UnixUid::new(1000),
            ))
            .admit(&ovis_terminal_authz::PeerEvidence::Remote(
                ovis_terminal_authz::RemoteTransport::Ssh,
            ))?,
            ovis_terminal_authz::Presentation::new(&self.socket.bearer, TimestampMs::new(1)),
        )?;
        Ok(self
            .socket
            .authority
            .issue(&owner, Issuance::new(request, TimestampMs::new(1)))?)
    }

    pub(crate) async fn close(self) -> TestResult {
        self.handler.inner().shutdown().await?;
        drop(self.handler);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still owned")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}

pub(crate) async fn call(client: &mut Client, method: &str, params: &Value) -> TestResult<Value> {
    client.send(&request(method, method, params)).await?;
    client.expect_next().await
}

pub(crate) async fn connected(rig: &Rig, bearer: &str) -> TestResult<Client> {
    let mut client = rig.socket.connect().await?;
    let hello = client.hello("1", bearer).await?;
    if hello.get("result").is_none() {
        return Err(format!("hello refused: {hello}").into());
    }
    Ok(client)
}
