//! The local runtime: real store, authority and dispatcher with the machine lane, served on
//! a private Unix socket and driven with literal NDJSON.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence,
    Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope, ScopeResource, UnixUid,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

use super::super::{MachineAdd, MachineGet, MachineLane, MachineRename};
use super::TestResult;
use super::remote::{bootstrap, identity, private_root};
use crate::dispatch::Dispatcher;
use crate::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};

type Core = Coordinator<OperationEvent, OperationProducer>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-00000000004a";

#[derive(Debug)]
pub(super) struct Local {
    root: PathBuf,
    authority: Arc<Authority<RegistryIdentities>>,
    core: Arc<Core>,
    dispatch: Arc<Dispatch>,
    server: ServerHandle,
    pub(super) bearer: String,
}

impl Local {
    pub(super) async fn start(bound: Duration) -> TestResult<Self> {
        let root = private_root()?;
        let runtime = identity(MACHINE, 1)?;
        let (authority, bearer) = bootstrap(runtime)?;
        let authority = Arc::new(authority);
        let store = Store::open(&root.join("state.db"), StoreOptions::default())?;
        let core = Arc::new(Core::new(
            store,
            runtime,
            OperationProducer::new(runtime.runtime_id()),
        ));
        let lane = MachineLane::new(runtime.machine_id(), bound);
        let mut dispatch = Dispatch::new(Arc::clone(&authority), runtime, Arc::clone(&core));
        dispatch.register(MachineAdd::new(&lane))?;
        dispatch.register(MachineGet::new(&lane))?;
        dispatch.register(MachineRename::new(&lane))?;
        let dispatch = Arc::new(dispatch);
        let endpoint = UnixEndpoint::bind(&root.join("s.sock")).await?;
        let config = ServerConfig::new(Arc::clone(&authority), runtime);
        let server = serve_unix(endpoint, config, Arc::clone(&dispatch))?;
        Ok(Self {
            root,
            authority,
            core,
            dispatch,
            server,
            bearer,
        })
    }

    /// A credential that may only observe this runtime.
    pub(super) fn observer(&self) -> TestResult<String> {
        let at = TimestampMs::new(1);
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(0)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let owner = self
            .authority
            .authenticate(&admission, Presentation::new(&self.bearer, at))?;
        let machine = identity(MACHINE, 1)?.machine_id();
        let grant = Grant::new(
            Permission::Observe,
            ResourceScope::new(machine, ScopeResource::Machine),
        );
        let request = IssueRequest::new(
            PrincipalId::parse("viewer")?,
            vec![grant],
            TimestampMs::new(u64::MAX),
        );
        Ok(self
            .authority
            .issue(&owner, Issuance::new(request, at))?
            .bearer()
            .to_owned())
    }

    pub(super) async fn client(&self, bearer: &str) -> TestResult<Client> {
        let mut client = Client(BufReader::new(
            UnixStream::connect(self.root.join("s.sock")).await?,
        ));
        let hello = client
            .call("hello", &json!({"generation": "1", "client": {"name": "t43", "version": "1"}, "credential": bearer}))
            .await?;
        if hello.pointer("/result/machine_id") != Some(&json!(MACHINE)) {
            return Err(format!("hello failed: {hello}").into());
        }
        Ok(client)
    }

    pub(super) async fn close(self) -> TestResult {
        self.server.shutdown().await?;
        self.dispatch.shutdown().await?;
        drop(self.dispatch);
        let core = Arc::try_unwrap(self.core).map_err(|_| "core still shared")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Client(BufReader<UnixStream>);

impl Client {
    pub(super) async fn call(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        let request = json!({"jsonrpc": "2.0", "id": method, "method": method, "params": params});
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        self.0.get_mut().write_all(&bytes).await?;
        let mut line = String::new();
        let read =
            tokio::time::timeout(Duration::from_secs(30), self.0.read_line(&mut line)).await??;
        if read == 0 {
            return Err("the connection ended before the reply".into());
        }
        Ok(serde_json::from_str(&line)?)
    }
}
