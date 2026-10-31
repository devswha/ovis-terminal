//! The fixture process: the real server crate, dispatcher, authority and SQLite store on a
//! private Unix socket in the isolated root, until its stdin closes.

use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_core::terminals::process_info::NativeTable;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::dispatch::lanes::process_info::ProcessInfoRead;
use ovis_terminal_server::listen::{ServerConfig, UnixEndpoint, serve_unix};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;

use super::documents::{DocGet, DocSet};
use super::handler::{Dispatch, FixtureHandler};
use super::processes::{ProcessDirectoryFixture, Programs};
use super::terminals::TerminalKeep;
use crate::error::RunnerError;

/// Machine identity every fixture reports in hello.
pub(crate) const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-000000000021";

/// Maps a failure at `stage` into the fixture error, keeping it as the source.
pub(crate) fn at<E>(stage: &'static str) -> impl FnOnce(E) -> RunnerError
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    move |source| RunnerError::Fixture {
        stage,
        source: source.into(),
    }
}

/// The runtime identity of every fixture run.
pub(crate) fn identity() -> Result<RuntimeIdentity, RunnerError> {
    let machine: MachineId = MACHINE.parse().map_err(at("parse the machine id"))?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(
            machine,
            HostBootId::try_from(1).map_err(at("build the host boot id"))?,
        ),
        RuntimeId::try_from(21).map_err(at("build the runtime id"))?,
        ServerBootId::try_from(1).map_err(at("build the server boot id"))?,
    ))
}

/// Serves until stdin reaches end of file, then prints the serve report and returns.
pub(crate) async fn serve(root: &Path) -> Result<(), RunnerError> {
    let runtime = identity()?;
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = PrincipalId::parse("owner").map_err(at("parse the owner principal"))?;
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(
        binding,
        OwnerSpec::new(owner, TimestampMs::new(u64::MAX)),
    ))
    .map_err(at("bootstrap the authority"))?;
    let authority = Arc::new(authority);
    let store = Store::open(&root.join("state.db"), StoreOptions::default())
        .map_err(at("open the store"))?;
    let core = Arc::new(Coordinator::new(
        store,
        runtime,
        OperationProducer::new(runtime.runtime_id()),
    ));
    let effects = root.join("effects");
    std::fs::create_dir(&effects).map_err(at("create the effect directory"))?;
    let mut dispatch = Dispatch::new(Arc::clone(&authority), runtime, Arc::clone(&core));
    dispatch
        .register(DocSet::new(&effects, runtime))
        .map_err(at("register doc.set"))?;
    dispatch
        .register(DocGet::new(&effects, runtime))
        .map_err(at("register doc.get"))?;
    dispatch
        .register(TerminalKeep::new(&effects, runtime)?)
        .map_err(at("register terminal.keep"))?;
    let agents = super::agents::register(&mut dispatch, runtime, Arc::clone(&core))?;
    let (owned_root, shared, service) = (
        root.to_path_buf(),
        Arc::clone(&authority),
        Arc::clone(&agents.0),
    );
    let bearer = issued.bearer().to_owned();
    let programs = tokio::task::spawn_blocking(move || {
        Programs::start(&owned_root, (runtime, shared, bearer), service)
    })
    .await
    .map_err(at("join the program launch"))??;
    dispatch
        .register_read(ProcessInfoRead::new(
            ProcessDirectoryFixture::new(Arc::clone(&programs), runtime)?,
            Arc::new(NativeTable::default()),
        ))
        .map_err(at("register terminal.process_info"))?;
    super::transcripts::register(&mut dispatch, agents, (runtime, root))?;
    let handler = Arc::new(FixtureHandler::new(
        Arc::new(dispatch),
        root.to_path_buf(),
        programs,
    ));
    let socket = root.join("run").join("s.sock");
    let endpoint = UnixEndpoint::bind(&socket)
        .await
        .map_err(at("bind the socket"))?;
    let config = ServerConfig::new(authority, runtime);
    let server = serve_unix(endpoint, config, Arc::clone(&handler)).map_err(at("serve"))?;
    announce(&json!({"ready": {"socket": socket, "bearer": issued.bearer()}}))?;
    let mut sink = Vec::new();
    tokio::io::stdin()
        .read_to_end(&mut sink)
        .await
        .map_err(at("wait for the runner to close stdin"))?;
    let report = server
        .shutdown()
        .await
        .map_err(at("shut the server down"))?;
    handler.shutdown().await?;
    drop(handler);
    let core = Arc::try_unwrap(core).map_err(|_| at("release the coordinator")("still shared"))?;
    tokio::task::spawn_blocking(move || core.close())
        .await
        .map_err(at("join the store close"))?
        .map_err(at("close the store"))?;
    let failures: serde_json::Map<String, Value> = report
        .failures()
        .iter()
        .map(|(kind, count)| ((*kind).to_owned(), json!(count.to_string())))
        .collect();
    announce(&json!({"report": {
        "accepted": report.accepted().to_string(),
        "failures": failures,
        "panicked": report.panicked().to_string(),
    }}))
}

fn announce(line: &Value) -> Result<(), RunnerError> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")
        .and_then(|()| stdout.flush())
        .map_err(|source| RunnerError::Io {
            operation: "announce to the runner",
            source,
        })
}
