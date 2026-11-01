//! The update lane over a real dispatcher, SQLite ledger, holder, restartable control and
//! Unix socket; release archives are written by the system `tar`.

use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Presentation, PrincipalId, RemoteTransport, RuntimeBinding, UnixUid,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, ProcessIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_core::restart::{Control, ControlPlan};
use ovis_terminal_core::terminals::{Call, Launch, Program};
use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::domain::update::{BuildTarget, HolderGeneration};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::{ProtocolGeneration, TimestampMs};
use ovis_terminal_services::update::{Config, CurrentBuild, Requirement, Updates};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::task::block_in_place;

use super::UpdateLane;
use super::control::ControlRestart;
use crate::dispatch::Dispatcher;
use crate::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};

mod wire;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;
const WAIT: Duration = Duration::from_secs(15);

struct Fixture {
    root: PathBuf,
    holder: Holder,
    process: ProcessIdentity,
    served: Option<(ServerHandle, Arc<Dispatch>)>,
    bearer: String,
}

fn identity() -> TestResult<RuntimeIdentity> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000047".parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(47)?,
        ServerBootId::try_from(1)?,
    ))
}

fn script(version: &str, status: u8) -> String {
    let report = json!({"binary": "ovis-terminal", "version": version, "generation": "1"});
    format!("#!/bin/sh\nprintf '%s\\n' '{report}'\nexit {status}\n")
}

/// Publishes a release of `version` whose executable exits `status`; returns its digest.
fn publish(root: &Path, version: &str, status: u8) -> TestResult<String> {
    let target = BuildTarget::host();
    let build = root.join("work").join(version);
    std::fs::create_dir_all(&build)?;
    let manifest = json!({"version": version, "target": target.as_str(),
        "protocol_generations": ["1"], "holder_generations": ["2"]});
    std::fs::write(build.join("build.json"), manifest.to_string())?;
    std::fs::write(build.join("ovis-terminal"), script(version, status))?;
    let dir = root.join("releases").join(version);
    std::fs::create_dir_all(&dir)?;
    let archive = dir.join(format!("ovis-terminal-{version}-{target}.tar"));
    let status = Command::new("tar")
        .args(["--format=ustar", "-cf"])
        .arg(&archive)
        .arg("-C")
        .arg(&build)
        .args(["build.json", "ovis-terminal"])
        .status()?;
    assert!(status.success(), "tar failed: {status}");
    let digest = Sha256::digest(std::fs::read(archive)?);
    Ok(digest
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect())
}

impl Fixture {
    /// A control holding one live child, and the update lane served on a socket.
    async fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt47-{}", uuid::Uuid::now_v7()));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        std::fs::create_dir(root.join("control"))?;
        std::fs::write(root.join("ovis-terminal-1"), script("1.0.0", 0))?;
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
                HolderBuildId::new("task-47-lane".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let plan = ControlPlan::new(
            root.join("control"),
            (holder.clone(), Arc::clone(&authority)),
            session,
        );
        let control = Arc::new(block_in_place(|| Control::open(plan, identity))?.0);
        let program = Program::new(
            vec!["/bin/sh".into(), "-c".into(), "read gate".into()],
            root.clone(),
            [("PATH".into(), "/usr/bin:/bin".into())].into(),
        )?;
        let launch = Launch::new(
            program,
            serde_json::from_value(json!({"cols": 80, "rows": 24}))?,
        )?;
        let record = block_in_place(|| {
            control.with_terminals(|terminals, session| {
                let admitted = terminals.run(Call::new(session, OpId::new(op_id())), launch)?;
                terminals.await_spawn(admitted.id(), WAIT)
            })
        })?;
        let process = record.lifecycle.process().ok_or("no process")?.clone();
        let config = Config::new(
            (root.join("install"), root.join("releases")),
            CurrentBuild::new("1.0.0".parse()?, root.join("ovis-terminal-1")),
            Requirement::new(ProtocolGeneration::new(1), HolderGeneration::new(2)),
        );
        let restart = Arc::new(ControlRestart::new(Arc::clone(&control)));
        let dispatch = block_in_place(|| -> TestResult<Dispatch> {
            let service = Arc::new(Updates::open(config, restart)?);
            let store = Store::open(&root.join("ledger"), StoreOptions::default())?;
            let core = Arc::new(Coordinator::new(
                store,
                identity,
                OperationProducer::new(identity.runtime_id()),
            ));
            let mut dispatch = Dispatch::new(Arc::clone(&authority), identity, core);
            UpdateLane::new(service, identity.machine_id()).register(&mut dispatch)?;
            Ok(dispatch)
        })?;
        let dispatch = Arc::new(dispatch);
        let endpoint = UnixEndpoint::bind(&root.join("s.sock")).await?;
        let server = serve_unix(
            endpoint,
            ServerConfig::new(authority, identity),
            Arc::clone(&dispatch),
        )?;
        Ok(Self {
            root,
            holder,
            process,
            served: Some((server, dispatch)),
            bearer,
        })
    }

    async fn connect(&self) -> TestResult<Client> {
        let mut client = Client(BufReader::new(
            UnixStream::connect(self.root.join("s.sock")).await?,
        ));
        let hello = json!({"generation": "1", "client": {"name": "t47", "version": "1"}, "credential": self.bearer});
        let reply = client.call("hello", &hello).await?;
        reply
            .get("result")
            .ok_or_else(|| format!("hello refused: {reply}"))?;
        Ok(client)
    }

    /// Whether the child still runs with the start token the holder reported at spawn.
    fn child_running(&self) -> TestResult<bool> {
        let pid = u32::try_from(self.process.pid().get())?;
        assert!(pid > 1, "only the spawned child is inspected");
        Ok(read_start_token(ProcessId::new(pid))
            .is_ok_and(|token| token.as_str() == self.process.start_token().as_str()))
    }

    async fn close(mut self) -> TestResult {
        let (server, dispatch) = self.served.take().ok_or("already closed")?;
        server.shutdown().await?;
        dispatch.shutdown().await?;
        block_in_place(|| drop((dispatch, self)));
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let stopped = self.holder.shutdown();
        assert!(stopped.is_ok(), "join owned holder workers: {stopped:?}");
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "remove owned fixture: {removed:?}");
    }
}

fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

struct Client(BufReader<UnixStream>);

impl Client {
    async fn call(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        let request = json!({"jsonrpc": "2.0", "id": method, "method": method, "params": params});
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        self.0.get_mut().write_all(&bytes).await?;
        let mut line = String::new();
        tokio::time::timeout(WAIT, self.0.read_line(&mut line)).await??;
        Ok(serde_json::from_str(&line)?)
    }
}
