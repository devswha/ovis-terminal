//! The fixture process: the real server crate, dispatcher, authority and SQLite store on a
//! private Unix socket in the isolated root, until its stdin closes.

use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, Grant, Issuance, IssueRequest, IssuedCredential,
    LocalAccount, OwnerSpec, PeerEvidence, Permission, Presentation, PrincipalId, RemoteTransport,
    ResourceScope, RuntimeBinding, ScopeResource, UnixUid,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::authn::Revalidated;
use ovis_terminal_server::dispatch::lanes::health::Measured;
use ovis_terminal_server::dispatch::lanes::prompt_bridge::{Gated, PromptBridgeGate};
use ovis_terminal_server::listen::{ServeReport, ServerConfig, UnixEndpoint, serve_unix};
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;

use super::handler::{Dispatch, Event, FixtureHandler};
use crate::error::RunnerError;

mod registration;

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

/// The owner's authority over `runtime`, and the owner bearer it issued.
fn bootstrap(
    runtime: RuntimeIdentity,
) -> Result<
    (
        Arc<Authority<RegistryIdentities>>,
        IssuedCredential<RegistryIdentities>,
    ),
    RunnerError,
> {
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = PrincipalId::parse("owner").map_err(at("parse the owner principal"))?;
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(
        binding,
        OwnerSpec::new(owner, TimestampMs::new(u64::MAX)),
    ))
    .map_err(at("bootstrap the authority"))?;
    Ok((Arc::new(authority), issued))
}

/// Serves until stdin reaches end of file, then prints the serve report and returns.
pub(crate) async fn serve(root: &Path) -> Result<(), RunnerError> {
    let runtime = identity()?;
    let (authority, issued) = bootstrap(runtime)?;
    let core = open_core(root, runtime)?;
    let mut dispatch = Dispatch::new(Arc::clone(&authority), runtime, Arc::clone(&core));
    let lanes = (&authority, issued.bearer(), &core);
    let registered = registration::register(&mut dispatch, (root, runtime), lanes).await?;
    let observer = delegate(&authority, issued.bearer(), ("observer", OBSERVER))?;
    let writer = delegate(&authority, issued.bearer(), ("writer", WRITER))?;
    let directory = Arc::clone(&registered.probes.directory);
    let health = Arc::clone(&registered.probes.health.health);
    let handler = Arc::new(FixtureHandler::new(
        Arc::new(dispatch),
        root.to_path_buf(),
        registered.probes,
    ));
    let gated = Gated::new(
        PromptBridgeGate::new(directory, (registered.status, registered.prompts)),
        Arc::clone(&handler),
    );
    let socket = root.join("run").join("s.sock");
    let endpoint = UnixEndpoint::bind(&socket)
        .await
        .map_err(at("bind the socket"))?;
    let measured = Measured::new(health, Arc::new(gated));
    let revalidated = Revalidated::new(Arc::clone(&authority), measured);
    let config = ServerConfig::new(authority, runtime);
    let server = serve_unix(endpoint, config, Arc::new(revalidated)).map_err(at("serve"))?;
    announce(&json!({"ready": {
        "socket": socket, "bearer": issued.bearer(), "observer": observer, "writer": writer,
    }}))?;
    runner_closed_stdin().await?;
    let report = server
        .shutdown()
        .await
        .map_err(at("shut the server down"))?;
    handler.shutdown().await?;
    drop(handler);
    let held = registered.held;
    tokio::task::spawn_blocking(move || held.shutdown())
        .await
        .map_err(at("join the update holder shutdown"))??;
    let core = Arc::try_unwrap(core).map_err(|_| at("release the coordinator")("still shared"))?;
    tokio::task::spawn_blocking(move || core.close())
        .await
        .map_err(at("join the store close"))?
        .map_err(at("close the store"))?;
    announce_report(&report)
}

/// Returns once the runner closes the fixture's stdin, its signal to stop serving.
async fn runner_closed_stdin() -> Result<(), RunnerError> {
    let mut sink = Vec::new();
    tokio::io::stdin()
        .read_to_end(&mut sink)
        .await
        .map_err(at("wait for the runner to close stdin"))?;
    Ok(())
}

/// The coordinator over the run's SQLite store, sequencing as `runtime`.
fn open_core(
    root: &Path,
    runtime: RuntimeIdentity,
) -> Result<Arc<Coordinator<Event, OperationProducer>>, RunnerError> {
    let store = Store::open(&root.join("state.db"), StoreOptions::default())
        .map_err(at("open the store"))?;
    Ok(Arc::new(Coordinator::new(
        store,
        runtime,
        OperationProducer::new(runtime.runtime_id()),
    )))
}

/// The serve report line: accepted connections, failures by kind, panics.
fn announce_report(report: &ServeReport) -> Result<(), RunnerError> {
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

/// May only observe the machine.
const OBSERVER: &[Permission] = &[Permission::Observe];

/// Observes, types and answers prompts on the machine; may neither take over nor
/// administer, and is the credential the rotation scenarios withdraw.
const WRITER: &[Permission] = &[
    Permission::Observe,
    Permission::Interact,
    Permission::PromptAnswer,
];

/// A bearer of `principal` holding `permissions` on the whole machine, issued by the owner.
fn delegate(
    authority: &Authority<RegistryIdentities>,
    owner: &str,
    (principal, permissions): (&str, &[Permission]),
) -> Result<String, RunnerError> {
    let failed = at("issue a delegated credential");
    let now = TimestampMs::new(1);
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))
        .map_err(at("admit the issuing owner"))?;
    let owner = authority
        .authenticate(&admission, Presentation::new(owner, now))
        .map_err(at("authenticate the issuing owner"))?;
    let scope = ResourceScope::new(identity()?.machine_id(), ScopeResource::Machine);
    let principal = PrincipalId::parse(principal).map_err(at("parse a delegated principal"))?;
    let grants = permissions
        .iter()
        .map(|permission| Grant::new(*permission, scope.clone()))
        .collect();
    let request = IssueRequest::new(principal, grants, TimestampMs::new(u64::MAX));
    let issued = authority
        .issue(&owner, Issuance::new(request, now))
        .map_err(failed)?;
    Ok(issued.bearer().to_owned())
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
