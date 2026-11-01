//! The local runtime under test: real authority and connection layer on a private Unix
//! socket, remote dispatch in front of a local handler that counts what reaches it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount,
    OwnerSpec, PeerEvidence, Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope,
    RuntimeBinding, ScopeResource, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RegistryIdentities, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_remote::actors::{Actors, Inbound, Limits};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::super::{RemoteDispatch, RemoteRoutes};
use super::TestResult;
use super::peer::{Mode, Peer, Pipe};
use crate::conn::{Call, RequestHandler};
use crate::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};

pub(super) const LOCAL: &str = "machine:0191f1a2-0000-7000-8000-000000000052";

/// Answers every call itself and counts them.
#[derive(Debug, Default)]
pub(super) struct Counting(AtomicUsize);

impl Counting {
    pub(super) fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl RequestHandler for Counting {
    fn methods(&self) -> Vec<&'static str> {
        vec!["input.send", "terminal.run", "terminal.get"]
    }

    fn call(&self, call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(json!({"local": call.request.method()})))
    }
}

pub(super) fn remote_id(last: u8) -> MachineId {
    MachineId::new(format!(
        "machine:0191f1a2-0000-7000-8000-0000000000{last:02x}"
    ))
}

#[derive(Debug)]
pub(super) struct Harness {
    root: PathBuf,
    authority: Arc<Authority<RegistryIdentities>>,
    pub(super) actors: Arc<Actors<Pipe>>,
    pub(super) local: Arc<Counting>,
    server: ServerHandle,
    pub(super) bearer: String,
    _inbound: mpsc::Receiver<Inbound>,
}

impl Harness {
    pub(super) async fn start(deadline: Duration) -> TestResult<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let index = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from(format!("/tmp/ovt52-{}-{index}", std::process::id()));
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&root)?;
        let runtime = RuntimeIdentity::new(
            HostIdentity::new(LOCAL.parse()?, HostBootId::try_from(1)?),
            RuntimeId::try_from(52)?,
            ServerBootId::try_from(1)?,
        );
        let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        let authority = Arc::new(authority);
        let (actors, inbound) = Actors::new(Limits::new(8, Duration::from_secs(60), 256));
        let actors = Arc::new(actors);
        let local = Arc::new(Counting::default());
        let routes = RemoteRoutes::new(
            (Arc::clone(&actors), Arc::clone(&authority)),
            runtime.machine_id(),
            deadline,
        );
        let dispatch = RemoteDispatch::new(routes, Arc::clone(&local));
        let endpoint = UnixEndpoint::bind(&root.join("s.sock")).await?;
        let config = ServerConfig::new(Arc::clone(&authority), runtime);
        let server = serve_unix(endpoint, config, Arc::new(dispatch))?;
        Ok(Self {
            root,
            authority,
            actors,
            local,
            server,
            bearer: issued.bearer().to_owned(),
            _inbound: inbound,
        })
    }

    /// Adds machine `name` and attaches a line served by a peer in `mode`.
    pub(super) async fn machine(&self, (name, last): (&str, u8), mode: Mode) -> TestResult<Peer> {
        let record = serde_json::from_value(json!({
            "machine_id": remote_id(last), "name": name, "aliases": [format!("{name}-old")],
            "state": "connecting",
            "remote_lifetime": "runtime:7", "remote_boot_id": "1", "last_seen_seq": null,
            "version": "1.0.0", "revision": "1",
        }))?;
        self.actors.add(record)?;
        let (peer, line) = Peer::start(mode);
        self.actors.attach(&remote_id(last), line).await?;
        Ok(peer)
    }

    /// A credential that may only observe this runtime.
    pub(super) fn observer(&self) -> TestResult<String> {
        let at = TimestampMs::new(1);
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(0)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let owner = self
            .authority
            .authenticate(&admission, Presentation::new(&self.bearer, at))?;
        let scope = ResourceScope::new(LOCAL.parse()?, ScopeResource::Machine);
        let request = IssueRequest::new(
            PrincipalId::parse("viewer")?,
            vec![Grant::new(Permission::Observe, scope)],
            TimestampMs::new(u64::MAX),
        );
        let issued = self.authority.issue(&owner, Issuance::new(request, at))?;
        Ok(issued.bearer().to_owned())
    }

    pub(super) async fn client(&self, bearer: &str) -> TestResult<Client> {
        let stream = UnixStream::connect(self.root.join("s.sock")).await?;
        let mut client = Client(BufReader::new(stream));
        let hello = json!({"generation": "1", "client": {"name": "t52", "version": "1"},
            "credential": bearer});
        let reply = client.call("hello", &hello).await?;
        if reply.pointer("/result/machine_id") != Some(&json!(LOCAL)) {
            return Err(format!("hello failed: {reply}").into());
        }
        Ok(client)
    }

    pub(super) async fn close(self) -> TestResult {
        self.server.shutdown().await?;
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Client(BufReader<UnixStream>);

impl Client {
    pub(super) async fn send(&mut self, method: &str, params: &Value) -> TestResult {
        let request = json!({"jsonrpc": "2.0", "id": method, "method": method, "params": params});
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        self.0.get_mut().write_all(&bytes).await?;
        Ok(())
    }

    pub(super) async fn notify(&mut self, method: &str, params: &Value) -> TestResult {
        let notification = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let mut bytes = serde_json::to_vec(&notification)?;
        bytes.push(b'\n');
        self.0.get_mut().write_all(&bytes).await?;
        Ok(())
    }

    pub(super) async fn reply(&mut self) -> TestResult<Value> {
        let mut line = String::new();
        let read = tokio::time::timeout(super::HANG, self.0.read_line(&mut line)).await??;
        if read == 0 {
            return Err("the connection ended before the reply".into());
        }
        Ok(serde_json::from_str(&line)?)
    }

    pub(super) async fn call(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        self.send(method, params).await?;
        self.reply().await
    }
}
