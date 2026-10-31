use std::error::Error;
use std::io::Write as _;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount,
    OwnerSpec, PeerEvidence, Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope,
    RuntimeBinding, ScopeResource, UnixUid,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RegistryIdentities, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_services::plugins::Plugins;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

use crate::dispatch::Dispatcher;
use crate::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;
type Core = Coordinator<OperationEvent, OperationProducer>;

fn now() -> TestResult<TimestampMs> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

/// The real server, dispatcher, authority and SQLite ledger with the plugin routes.
pub(super) struct Harness {
    pub(super) root: PathBuf,
    authority: Arc<Authority<RegistryIdentities>>,
    pub(super) owner: String,
    core: Arc<Core>,
    dispatch: Arc<Dispatch>,
    server: ServerHandle,
}

impl Harness {
    /// Serves `manifests` (plugin id to manifest) whose `$ROOT` text names the test root.
    pub(super) async fn start(name: &str, manifests: &[(&str, Value)]) -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt46s-{}-{name}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&root)?;
        for (id, manifest) in manifests {
            let directory = root.join("plugins").join(id);
            std::fs::create_dir_all(&directory)?;
            let text = manifest
                .to_string()
                .replace("$ROOT", &root.to_string_lossy());
            std::fs::write(directory.join("plugin.json"), text)?;
        }
        let machine = "machine:0191f1a2-0000-7000-8000-000000000046".parse()?;
        let runtime = RuntimeIdentity::new(
            HostIdentity::new(machine, HostBootId::try_from(1)?),
            RuntimeId::try_from(46)?,
            ServerBootId::try_from(1)?,
        );
        let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        let authority = Arc::new(authority);
        let store = Store::open(&root.join("state.db"), StoreOptions::default())?;
        let core = Arc::new(Coordinator::new(
            store,
            runtime,
            OperationProducer::new(runtime.runtime_id()),
        ));
        let mut dispatch = Dispatch::new(Arc::clone(&authority), runtime, Arc::clone(&core));
        let plugins = Arc::new(Plugins::new(root.join("plugins")));
        let (list, invoke, run_get) = super::super::routes(&plugins, runtime.machine_id());
        dispatch.register(list)?;
        dispatch.register(invoke)?;
        dispatch.register(run_get)?;
        let dispatch = Arc::new(dispatch);
        let endpoint = UnixEndpoint::bind(&root.join("s.sock")).await?;
        let config = ServerConfig::new(Arc::clone(&authority), runtime);
        let server = serve_unix(endpoint, config, Arc::clone(&dispatch))?;
        Ok(Self {
            root,
            authority,
            owner: issued.bearer().to_owned(),
            core,
            dispatch,
            server,
        })
    }

    /// Issues a credential holding only `permission` over the whole machine.
    pub(super) fn issue(&self, principal: &str, permission: Permission) -> TestResult<String> {
        let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)));
        let admission = policy.admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let owner = self
            .authority
            .authenticate(&admission, Presentation::new(&self.owner, now()?))?;
        let machine = "machine:0191f1a2-0000-7000-8000-000000000046".parse()?;
        let grant = Grant::new(
            permission,
            ResourceScope::new(machine, ScopeResource::Machine),
        );
        let request = IssueRequest::new(
            PrincipalId::parse(principal)?,
            vec![grant],
            TimestampMs::new(u64::MAX),
        );
        let issued = self
            .authority
            .issue(&owner, Issuance::new(request, now()?))?;
        Ok(issued.bearer().to_owned())
    }

    pub(super) async fn connect(&self, bearer: &str) -> TestResult<Client> {
        let mut client = Client {
            stream: BufReader::new(UnixStream::connect(self.root.join("s.sock")).await?),
            sent: 0,
        };
        client
            .send(
                &json!({"jsonrpc": "2.0", "id": "hello", "method": "hello", "params": {
                "generation": "1", "client": {"name": "plugins-test", "version": "1"},
                "credential": bearer}}),
            )
            .await?;
        let hello = client.next().await?;
        if hello.get("result").is_none() {
            return Err(format!("hello refused: {hello}").into());
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

pub(super) struct Client {
    stream: BufReader<UnixStream>,
    sent: u64,
}

pub(super) fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

impl Client {
    pub(super) async fn send(&mut self, message: &Value) -> TestResult {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.stream.get_mut().write_all(&bytes).await?;
        if at(message, "/method") != json!("hello") {
            writeln!(std::io::stdout(), "-> {message}")?;
        }
        Ok(())
    }

    pub(super) async fn next(&mut self) -> TestResult<Value> {
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(30), self.stream.read_line(&mut line))
            .await??;
        if read == 0 {
            return Err("connection ended before a reply".into());
        }
        writeln!(std::io::stdout(), "<- {}", line.trim_end())?;
        Ok(serde_json::from_str(&line)?)
    }

    pub(super) async fn call(&mut self, method: &str, params: Value) -> TestResult<Value> {
        self.sent = self.sent.saturating_add(1);
        let id = format!("{method}-{}", self.sent);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        let reply = self.next().await?;
        if at(&reply, "/id") != json!(id) {
            return Err(format!("reply for another request: {reply}").into());
        }
        Ok(reply)
    }
}

pub(super) fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
