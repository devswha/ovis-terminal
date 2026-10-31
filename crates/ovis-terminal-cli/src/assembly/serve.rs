//! Starting, announcing, serving until told to stop, and stopping in reverse order:
//! connections, attach deliveries, the runtime thread, then the lease log.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::listen::{
    EndpointDescriptor, EndpointEnvironment, Platform, ServerConfig, UnixEndpoint, serve_unix,
};
use ovis_terminal_server::streams::{
    AttachHandler, AttachLane, LeaseJournal, SessionBacklog, StreamLimits, StreamSetup,
};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::signal::unix::{SignalKind, signal};

use super::actor::{Actor, Runtime};
use super::boot::{LeaseLog, boot};
use super::directory::Directory;
use super::handler::{Assembled, Served};
use super::{AssemblyError, ServePlan};

/// Session-wide bound on bytes queued for all attach viewers.
const SESSION_BACKLOG_BYTES: u64 = 64 << 20;

fn announce(line: &Value) -> Result<(), AssemblyError> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")
        .and_then(|()| stdout.flush())
        .map_err(AssemblyError::Announce)
}

fn paths(plan: &ServePlan, env: &EndpointEnvironment) -> Result<(PathBuf, PathBuf), AssemblyError> {
    if let Some(root) = &plan.root {
        return Ok((root.join("run").join("s.sock"), root.join("state")));
    }
    let descriptor =
        EndpointDescriptor::resolve(env, Platform::CURRENT).map_err(AssemblyError::Endpoint)?;
    let socket = descriptor
        .unix_path()
        .ok_or_else(|| AssemblyError::start("resolve a Unix endpoint")("no Unix socket path"))?
        .to_path_buf();
    let mut state = socket.clone().into_os_string();
    state.push(".state");
    Ok((socket, PathBuf::from(state)))
}

fn private_dir(path: &std::path::Path) -> Result<(), AssemblyError> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(AssemblyError::start("create the state directory"))
}

async fn stopped(stdin_lifetime: bool) -> Result<(), AssemblyError> {
    let mut terminate =
        signal(SignalKind::terminate()).map_err(AssemblyError::start("watch SIGTERM"))?;
    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(AssemblyError::start("watch SIGINT"))?;
    let stdin = async {
        if stdin_lifetime {
            let mut sink = Vec::new();
            match tokio::io::stdin().read_to_end(&mut sink).await {
                Ok(_) | Err(_) => {}
            }
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
        () = stdin => {}
    }
    Ok(())
}

/// Runs one server lifetime for [`super::serve`].
pub(super) async fn run(plan: &ServePlan, env: &EndpointEnvironment) -> Result<(), AssemblyError> {
    let (socket, state) = paths(plan, env)?;
    private_dir(&state)?;
    let booted = boot(&state)?;
    let directory = Arc::new(Directory::new(booted.identity.machine_id()));
    let lane = AttachLane::new(Arc::clone(&directory));
    let limits = StreamLimits::advertised(&Limits::v1());
    let backlog = Arc::new(SessionBacklog::new(SESSION_BACKLOG_BYTES));
    let journal: Arc<dyn LeaseJournal> = Arc::<LeaseLog>::clone(&booted.leases);
    let setup = StreamSetup::new(limits, backlog, journal);
    let runtime = Runtime {
        terminals: booted.terminals,
        holder: booted.holder,
        identity: booted.identity,
        authority: Arc::clone(&booted.authority),
        lane: Arc::clone(&lane),
        directory,
        setup,
        geometry_log: booted.geometry,
        live: BTreeMap::new(),
        launches: BTreeMap::new(),
        created: BTreeMap::new(),
        started: Instant::now(),
    };
    let (actor, thread) = Actor::spawn(runtime)?;
    let served = Arc::new(Served {
        actor,
        lane: Arc::clone(&lane),
    });
    let handler = Arc::new(Assembled {
        attach: AttachHandler::new(Arc::clone(&served), Arc::clone(&lane)),
        served,
    });
    let endpoint = UnixEndpoint::bind(&socket)
        .await
        .map_err(AssemblyError::Endpoint)?;
    let config = ServerConfig::new(booted.authority, booted.identity);
    let listener =
        serve_unix(endpoint, config, Arc::clone(&handler)).map_err(AssemblyError::Endpoint)?;
    announce(&json!({"ready": {
        "socket": socket,
        "bearer": booted.bearers.owner,
        "observer": booted.bearers.observer,
    }}))?;
    let waited = stopped(plan.stdin_lifetime).await;
    let report = listener
        .shutdown()
        .await
        .map_err(AssemblyError::stop("close the endpoint"))?;
    waited?;
    lane.shutdown()
        .await
        .map_err(AssemblyError::stop("end attach deliveries"))?;
    drop(handler);
    drop(lane);
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .map_err(AssemblyError::stop("join the runtime thread"))??;
    let leases = Arc::try_unwrap(booted.leases)
        .map_err(|_| AssemblyError::stop("release the lease log")("still shared"))?;
    tokio::task::spawn_blocking(move || leases.close())
        .await
        .map_err(AssemblyError::stop("join the lease log close"))?
        .map_err(AssemblyError::stop("close the lease log"))?;
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
