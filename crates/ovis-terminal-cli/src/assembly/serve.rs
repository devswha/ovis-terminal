//! Starting, announcing, serving until told to stop, handing every listener and log over to
//! the successor boot on a planned restart, and stopping in reverse order.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::listen::{EndpointDescriptor, EndpointEnvironment, Platform};
use ovis_terminal_server::streams::{AttachLane, SessionBacklog, StreamLimits, StreamSetup};
use ovis_terminal_server::ws::WsServerHandle;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::signal::unix::{SignalKind, signal};

use super::actor::{Actor, Runtime};
use super::boot::boot;
use super::directory::Directory;
use super::generation::{Generation, Shared};
use super::journal::BootJournal;
use super::lifetime::Lifetime;
use super::recovery::Started;
use super::restart::{RestartSignal, StopSignal};
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

async fn stopped(stdin_lifetime: bool, stop: StopSignal) -> Result<(), AssemblyError> {
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
        () = stop.wait() => {}
    }
    Ok(())
}

/// The ready line: endpoint, bearers, and what this boot recovered or listens on.
fn ready(
    socket: &std::path::Path,
    bearers: &super::boot::Bearers,
    (started, websocket): (&Started, Option<&WsServerHandle>),
) -> Result<Value, AssemblyError> {
    let mut ready = json!({
        "socket": socket,
        "bearer": bearers.owner,
        "observer": bearers.observer,
        "writer": bearers.writer,
    });
    if let Value::Object(fields) = &mut ready {
        if let Some(report) = &started.recovery {
            let report = serde_json::to_value(report)
                .map_err(AssemblyError::start("encode the recovery report"))?;
            fields.insert("recovery".to_owned(), report);
        }
        if let Some(warning) = &started.warning {
            let warning = serde_json::to_value(warning)
                .map_err(AssemblyError::start("encode the start-up warning"))?;
            fields.insert("warnings".to_owned(), json!([warning]));
        }
        if let Some(listener) = websocket {
            fields.insert(
                "websocket".to_owned(),
                json!(listener.local_addr().to_string()),
            );
        }
    }
    Ok(json!({ "ready": ready }))
}

/// The line announcing that a successor boot serves the same endpoint.
fn restarted(generation: &Generation) -> Value {
    json!({"restarted": {
        "server_boot_id": generation.identity.server_boot_id().to_string(),
        "runtime_lifetime": generation.identity.runtime_id().to_string(),
    }})
}

/// Runs one server lifetime for [`super::serve`].
/// Refuses QA seams unless `--root` is a QA fixture root (it holds a `qa-fixture` file).
fn qa_seams_allowed(plan: &ServePlan) -> Result<(), AssemblyError> {
    let seams = plan.qa.holder_poll_ms.is_some()
        || plan.qa.actor_hold.is_some()
        || plan.qa.output_ring_bytes.is_some();
    let qa_root = plan
        .root
        .as_ref()
        .is_some_and(|root| root.join("qa-fixture").is_file());
    if seams && !qa_root {
        return Err(AssemblyError::start("accept the QA seams")(
            super::QaRootRequired,
        ));
    }
    Ok(())
}

/// Serves `generation` until a stop, handing over to each successor boot control
/// announces as completed, and returns the last generation, the old listeners' reports
/// and how the stop came.
async fn until_stopped(
    shared: &Shared,
    mut generation: Generation,
    stdin_lifetime: bool,
) -> Result<
    (
        Generation,
        Vec<ovis_terminal_server::listen::ServeReport>,
        Result<(), AssemblyError>,
    ),
    AssemblyError,
> {
    let mut completions = super::restart::completions(&shared.control)?;
    let stopped = stopped(stdin_lifetime, shared.stop.clone());
    tokio::pin!(stopped);
    let mut reports = Vec::new();
    let waited = loop {
        tokio::select! {
            waited = &mut stopped => break waited,
            Some(_completed) = completions.recv() => {
                // The restarting call's reply is written before its connection closes.
                shared.restart.quiet().await;
                let op = shared
                    .restart
                    .take()
                    .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
                let (next, ended) = generation.hand_over(shared, op).await?;
                if let Some(report) = ended {
                    reports.push(report);
                    announce(&restarted(&next))?;
                }
                generation = next;
            }
        }
    };
    Ok((generation, reports, waited))
}

pub(super) async fn run(plan: &ServePlan, env: &EndpointEnvironment) -> Result<(), AssemblyError> {
    qa_seams_allowed(plan)?;
    let (socket, state) = paths(plan, env)?;
    private_dir(&state)?;
    let booted = boot(&state).await?;
    let recovery =
        super::recovery::recover_boot(&state, &booted.control, (&booted.boot, booted.inherited))
            .await?;
    let directory = Arc::new(Directory::new(booted.identity.machine_id()));
    directory.recorded(&booted.recorded);
    let agents = super::lanes::agents::assemble(booted.identity, &directory)?;
    let lane = AttachLane::new(Arc::clone(&directory));
    let journal = Arc::new(BootJournal::new(&state, booted.leases));
    let limits = StreamLimits::advertised(&Limits::v1());
    let backlog = Arc::new(SessionBacklog::new(SESSION_BACKLOG_BYTES));
    let health = super::health::open(&state, booted.identity, recovery.recovery.clone())?;
    let setup = StreamSetup::new(limits, backlog, Arc::<BootJournal>::clone(&journal))
        .reporting_to(Arc::clone(&health));
    let mut runtime = Runtime {
        control: Arc::clone(&booted.control),
        poll: std::time::Duration::from_millis(plan.qa.holder_poll_ms.unwrap_or(5)),
        qa_hold: plan.qa.actor_hold.clone(),
        screen_bounds: super::live::screen_bounds(plan.qa.output_ring_bytes)?,
        journal: Arc::clone(&journal),
        status_feed: None,
        holder: booted.holder,
        identity: booted.identity,
        authority: Arc::clone(&booted.authority),
        lane: Arc::clone(&lane),
        directory: Arc::clone(&directory),
        setup,
        geometry_log: booted.geometry,
        layout_log: ovis_terminal_core::events::Coordinator::new(
            super::boot::store(&state, "layout.db")?,
            booted.identity,
            ovis_terminal_core::layout::Producer::new(booted.identity.runtime_id()),
        ),
        live: BTreeMap::new(),
        launches: BTreeMap::new(),
        created: BTreeMap::new(),
        status: Arc::clone(agents.status()),
        health: Arc::clone(&health),
        census: None,
    };
    runtime.publish_census();
    let (actor, thread) = Actor::spawn(runtime)?;
    let mut shared = Shared {
        state,
        socket,
        websocket_port: plan.websocket_port,
        authority: booted.authority,
        directory,
        lane,
        actor,
        control: booted.control,
        agents,
        journal,
        stop: StopSignal::default(),
        restart: RestartSignal::default(),
    };
    let generation = Generation::open(&shared, (booted.identity, health, None)).await?;
    shared.websocket_port = generation.websocket_port();
    let websocket = generation.websocket.as_ref();
    announce(&ready(
        &shared.socket,
        &booted.bearers,
        (&recovery, websocket),
    )?)?;
    let (generation, reports, waited) =
        until_stopped(&shared, generation, plan.stdin_lifetime).await?;
    let lifetime = Lifetime {
        generation,
        shared,
        thread,
        reports,
    };
    announce(&lifetime.stop(waited).await?)
}
