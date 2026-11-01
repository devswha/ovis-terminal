//! One measured run: spawn and subscribe everything, await readiness, apply the load
//! until every sample is taken, then settle the account.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_server::streams::scheduler::TerminalHandle;
use tokio::task::JoinSet;

use super::account::{self, Raw};
use super::delivery::{Client, Spec};
use super::error::Terminal50Error;
use super::fleet::{self, Fleet};
use super::host::Binary;
use super::lanes::{self, LaneSamples, Progress};
use super::measure;
use super::plan::Plan;
use super::report::Report;
use super::settle::{self, Load};

/// Bound on every terminal showing its first output.
const READY: Duration = Duration::from_secs(30);

/// Runs `plan` on a runtime of its own; `binary` is recorded in the report.
pub(super) fn run(plan: &Plan, binary: Binary) -> Result<(Report, Raw), Terminal50Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(plan.workers)
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    let measured = runtime.block_on(measure(plan, handle));
    runtime.shutdown_timeout(Duration::from_secs(5));
    Ok(account::report(plan, binary, &measured?))
}

fn clients(
    plan: &Plan,
    handles: &[TerminalHandle],
    progress: &Arc<Progress>,
) -> std::io::Result<Vec<Client>> {
    (0..=plan.shape.clients)
        .map(|index| {
            let spec = Spec {
                index,
                reads: index < plan.shape.clients,
                delay: plan.writer_delay(),
                progress: Arc::clone(progress),
            };
            Client::start(
                spec,
                handles.iter().map(TerminalHandle::subscribe).collect(),
            )
        })
        .collect()
}

async fn ready(fleet: &mut Fleet) -> Result<(), Terminal50Error> {
    let deadline = tokio::time::Instant::now()
        .checked_add(READY)
        .unwrap_or_else(tokio::time::Instant::now);
    for (terminal, signal) in fleet.ready.drain(..).enumerate() {
        let shown = tokio::time::timeout_at(deadline, signal).await;
        if !matches!(shown, Ok(Ok(()))) {
            return Err(Terminal50Error::NotReady {
                terminal,
                bound_ms: u64::try_from(READY.as_millis()).unwrap_or(u64::MAX),
            });
        }
    }
    Ok(())
}

async fn measure(plan: &Plan, runtime: tokio::runtime::Handle) -> Result<Load, Terminal50Error> {
    let history_root = plan.root.join("history");
    std::fs::create_dir_all(&history_root)?;
    let rss_before = measure::rss(&[])?;
    let mut fleet = fleet::spawn(plan, &history_root, &runtime)?;
    let progress = Arc::new(Progress::new(plan));
    let clients = clients(plan, &fleet.handles, &progress)?;
    ready(&mut fleet).await?;
    let pids = fleet.pids.clone();
    let started = Instant::now();
    let handles: Arc<[TerminalHandle]> = fleet.handles.iter().cloned().collect();
    let mut running = JoinSet::new();
    for index in 0..plan.shape.inflight {
        running.spawn(lanes::lane(
            index,
            Arc::clone(&handles),
            Arc::clone(&progress),
        ));
    }
    drop(handles);
    let mut samples = LaneSamples::default();
    let mut lanes = 0_usize;
    let mut rss_peak = rss_before;
    let mut sampling = tokio::time::interval(Duration::from_millis(250));
    while !running.is_empty() {
        tokio::select! {
            joined = running.join_next() => {
                if let Some(lane) = joined {
                    let lane = lane.map_err(std::io::Error::other)?;
                    lanes = lanes.saturating_add(usize::from(lane.sampled()));
                    samples.absorb(lane);
                }
            }
            _ = sampling.tick() => rss_peak = rss_peak.max(measure::rss(&pids)?),
        }
    }
    let wall = started.elapsed();
    rss_peak = rss_peak.max(measure::rss(&pids)?);
    let cpu = measure::cpu(&pids)?;
    let settled = settle::settle(plan, fleet, clients).await?;
    Ok(Load {
        samples,
        wanted: progress.wanted(),
        lanes,
        rss_before,
        rss_peak,
        disk: measure::disk_bytes(&history_root)?,
        cpu,
        wall,
        settled,
    })
}
