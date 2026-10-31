//! The restart lane over a real dispatcher, SQLite ledger, holder and Unix socket.
//!
//! A control-server process replacement is emulated in one test process: the listener,
//! dispatcher and ledger of the old boot are shut down and served again under the boot
//! identity the restarted control reports, while the holder and its programs stay.

mod wire;

use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Presentation, PrincipalId, RemoteTransport, RuntimeBinding, UnixUid,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, ProcessIdentity, RegistryIdentities, RuntimeId,
    RuntimeIdentity, ServerBootId,
};
use ovis_terminal_core::restart::{Control, ControlPlan};
use ovis_terminal_core::terminals::{Call, Launch, Program, Record};
use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::HelloResult;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::task::block_in_place;

use super::{RestartLane, ServerRestart, ServerStop};
use crate::dispatch::Dispatcher;
use crate::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;
const WAIT: Duration = Duration::from_secs(15);

struct Fixture {
    root: PathBuf,
    holder: Holder,
    authority: Arc<Authority<RegistryIdentities>>,
    bearer: String,
    control: Arc<Control>,
}

/// One served boot: its listener, dispatcher and ledger.
struct Served {
    server: ServerHandle,
    dispatch: Arc<Dispatch>,
}

fn identity() -> TestResult<RuntimeIdentity> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000048".parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(7)?,
        ServerBootId::try_from(1)?,
    ))
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt48-{}", uuid::Uuid::now_v7()));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        std::fs::create_dir(root.join("control"))?;
        let identity = identity()?;
        let binding = RuntimeBinding::new(identity.machine_id(), identity.runtime_id());
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        let authority = Arc::new(authority);
        let bearer = issued.bearer().to_owned();
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let session =
            authority.authenticate(&admission, Presentation::new(&bearer, TimestampMs::new(1)))?;
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("task-48-lane".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let plan = ControlPlan::new(
            root.join("control"),
            (holder.clone(), Arc::clone(&authority)),
            session,
        );
        let (control, _) = Control::open(plan, identity)?;
        Ok(Self {
            root,
            holder,
            authority,
            bearer,
            control: Arc::new(control),
        })
    }

    /// Serves the control's current boot on the fixture socket.
    async fn serve(&self) -> TestResult<Served> {
        let identity = self.control.identity()?;
        let dispatch = block_in_place(|| -> TestResult<Dispatch> {
            let store = Store::open(&self.root.join("ledger"), StoreOptions::default())?;
            let producer = OperationProducer::new(identity.runtime_id());
            let core = Arc::new(Coordinator::new(store, identity, producer));
            let mut dispatch = Dispatch::new(Arc::clone(&self.authority), identity, core);
            let lane = RestartLane::new(Arc::clone(&self.control), identity.machine_id());
            dispatch.register(ServerRestart::new(&lane))?;
            dispatch.register(ServerStop::new(&lane))?;
            Ok(dispatch)
        })?;
        let dispatch = Arc::new(dispatch);
        let endpoint = UnixEndpoint::bind(&self.root.join("s.sock")).await?;
        let config = ServerConfig::new(Arc::clone(&self.authority), identity);
        let server = serve_unix(endpoint, config, Arc::clone(&dispatch))?;
        Ok(Served { server, dispatch })
    }

    /// Connects and completes hello, returning the identity the server reported.
    async fn connect(&self) -> TestResult<(Client, HelloResult)> {
        let mut client = Client(BufReader::new(
            UnixStream::connect(self.root.join("s.sock")).await?,
        ));
        let hello = json!({"generation": "1", "client": {"name": "t48", "version": "1"}, "credential": self.bearer});
        let reply = client.call("hello", &hello).await?;
        let result = reply
            .get("result")
            .cloned()
            .ok_or_else(|| format!("hello refused: {reply}"))?;
        Ok((client, serde_json::from_value(result)?))
    }

    /// Runs `work` on the control's lifecycle actor off the async executor.
    fn terminals<R>(
        &self,
        work: impl FnOnce(
            &mut ovis_terminal_core::terminals::Terminals,
            &ovis_terminal_authz::Session,
        ) -> Result<R, ovis_terminal_core::terminals::TerminalError>,
    ) -> TestResult<R> {
        Ok(block_in_place(|| self.control.with_terminals(work))?)
    }

    fn spawn(&self, script: &str) -> TestResult<Record> {
        let program = Program::new(
            vec!["/bin/sh".into(), "-c".into(), script.into()],
            self.root.clone(),
            [("PATH".into(), "/usr/bin:/bin".into())].into(),
        )?;
        let launch = Launch::new(
            program,
            serde_json::from_value(json!({"cols": 80, "rows": 24}))?,
        )?;
        self.terminals(|terminals, session| {
            let op = OpId::new(uuid::Uuid::now_v7().to_string());
            let admitted = terminals.run(Call::new(session, op), launch)?;
            terminals.await_spawn(admitted.id(), WAIT)
        })
    }
}

impl Served {
    /// Ends this boot's listener and dispatcher, as the old control process exits.
    async fn retire(self) -> TestResult {
        self.server.shutdown().await?;
        release(self.dispatch).await
    }

    /// Ends this boot for a planned restart: connections close `server_restart`.
    async fn hand_over(self) -> TestResult {
        self.server.restart().await?;
        release(self.dispatch).await
    }
}

async fn release(dispatch: Arc<Dispatch>) -> TestResult {
    dispatch.shutdown().await?;
    block_in_place(|| drop(dispatch));
    Ok(())
}

impl Drop for Fixture {
    /// Stops the holder and removes the root; tests drop it through [`block_in_place`].
    fn drop(&mut self) {
        let stopped = self.holder.shutdown();
        assert!(stopped.is_ok(), "join owned holder workers: {stopped:?}");
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "remove owned fixture: {removed:?}");
    }
}

fn request(id: &str, method: &str, params: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Whether the OS still runs `process` with the start token the holder reported.
fn running(process: &ProcessIdentity) -> TestResult<bool> {
    let pid = u32::try_from(process.pid().get())?;
    assert!(pid > 1, "only a spawned child is inspected");
    Ok(read_start_token(ProcessId::new(pid))
        .is_ok_and(|token| token.as_str() == process.start_token().as_str()))
}

struct Client(BufReader<UnixStream>);

impl Client {
    async fn send(&mut self, message: &Value) -> TestResult {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.0.get_mut().write_all(&bytes).await?;
        Ok(())
    }

    /// The next record, or `None` once the server closed the connection.
    async fn record(&mut self) -> TestResult<Option<Value>> {
        let mut line = String::new();
        let read = tokio::time::timeout(WAIT, self.0.read_line(&mut line)).await??;
        Ok((read > 0)
            .then(|| serde_json::from_str(&line))
            .transpose()?)
    }

    async fn next(&mut self) -> TestResult<Value> {
        self.record()
            .await?
            .ok_or_else(|| "connection ended before a reply".into())
    }

    async fn call(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        self.send(&request(method, method, params)).await?;
        self.next().await
    }
}
