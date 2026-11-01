//! A load run: real PTY emitters, terminal actors, 16 request lanes, eight control-event
//! clients per terminal, optional viewers (one per terminal never reads), RSS sampling.

use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::scheduler::{
    ActorConfig, ActorTask, OutputEvent, TerminalHandle, TerminalSetup,
};
use ovis_terminal_vt::ParseBudget;
use serde::Serialize;
use tokio::sync::broadcast::Receiver;
use tokio::task::JoinSet;

use super::clients::{self, LaneSamples};
use super::error::TuningError;
use super::fleet::{Fleet, spawn_fleet};
use super::measure::{disk_bytes, rss};
use super::pty::Emit;

const LANES: usize = 16;
const CLIENTS: usize = 8;

/// The shape of one run.
#[derive(Debug, Clone, Copy)]
pub(super) struct Load {
    pub(super) terminals: usize,
    pub(super) viewers: usize,
    pub(super) bytes_per_second: usize,
    pub(super) seconds: u64,
    pub(super) parse_budget: Option<NonZeroUsize>,
    pub(super) workers: Option<NonZeroUsize>,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct LoadReport {
    terminals: usize,
    viewers: usize,
    workers: usize,
    lanes: usize,
    clients: usize,
    aggregate_bytes_per_second: usize,
    seconds: u64,
    parse_budget_bytes: usize,
    admissions: usize,
    controls: usize,
    rejected: u64,
    admission_p50_ms: f64,
    admission_p99_ms: f64,
    admission_max_ms: f64,
    control_max_ms: f64,
    events: usize,
    event_max_ms: f64,
    events_lost: u64,
    pumped_bytes: u64,
    parsed_bytes: u64,
    parse_turns: u64,
    max_turn_ms: f64,
    output_waits: u64,
    input_unsent_entries: usize,
    viewer_bytes: u64,
    viewer_events_lost: u64,
    stalled_viewers: usize,
    stalled_events_lost: u64,
    rss_before_bytes: u64,
    rss_peak_bytes: u64,
    rss_after_bytes: u64,
    history_disk_bytes: u64,
    history_recorded_bytes: u64,
    history_evicted_segments: u64,
    history_faults: u64,
}

pub(super) fn run(exe: &Path, load: Load) -> Result<LoadReport, TuningError> {
    let workers = load.workers.map_or_else(
        || {
            std::thread::available_parallelism()
                .map_or(2, NonZeroUsize::get)
                .clamp(2, 8)
        },
        NonZeroUsize::get,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    let mut report = runtime.block_on(drive(exe, load, handle))?;
    report.workers = workers;
    Ok(report)
}

async fn drive(
    exe: &Path,
    load: Load,
    runtime: tokio::runtime::Handle,
) -> Result<LoadReport, TuningError> {
    let limits = Limits::v1();
    let budget = load
        .parse_budget
        .map_or(ParseBudget::DEFAULT, ParseBudget::new);
    let config = ActorConfig::advertised(&limits).with_parse_budget(budget);
    let setup = TerminalSetup::advertised(config, &limits);
    let mut report = LoadReport {
        terminals: load.terminals,
        viewers: load.viewers,
        lanes: LANES,
        clients: CLIENTS,
        aggregate_bytes_per_second: load.bytes_per_second,
        seconds: load.seconds,
        parse_budget_bytes: budget.bytes(),
        rss_before_bytes: rss(&[])?,
        ..LoadReport::default()
    };
    let emit = Emit {
        bytes_per_second: load
            .bytes_per_second
            .checked_div(load.terminals)
            .unwrap_or(0),
        seconds: load.seconds,
    };
    let Fleet {
        handles,
        tasks,
        ptys,
        pids,
        viewers,
        stalled,
        history_root,
    } = spawn_fleet(exe, (load, emit, &setup), &runtime)?;
    let mut watchers = JoinSet::new();
    for _ in 0..CLIENTS {
        for handle in &handles {
            watchers.spawn(clients::watch(handle.subscribe()));
        }
    }
    let run_for = Duration::from_secs(load.seconds);
    let until = Instant::now()
        .checked_add(run_for)
        .unwrap_or_else(Instant::now);
    let mut lanes = JoinSet::new();
    for index in 0..LANES {
        lanes.spawn(clients::lane(index, handles.clone(), until));
    }
    let mut samples = LaneSamples::default();
    let mut peak = report.rss_before_bytes;
    let mut sampling = tokio::time::interval(Duration::from_millis(250));
    while !lanes.is_empty() {
        tokio::select! {
            joined = lanes.join_next() => {
                if let Some(Ok(lane)) = joined {
                    samples.admission.extend(lane.admission);
                    samples.control.extend(lane.control);
                    samples.rejected = samples.rejected.saturating_add(lane.rejected);
                }
            }
            _ = sampling.tick() => peak = peak.max(rss(&pids)?),
        }
    }
    let mut forwarders = Vec::new();
    for pty in ptys {
        let (pumped, forwarder) = tokio::task::spawn_blocking(move || pty.finish())
            .await
            .map_err(std::io::Error::other)??;
        report.pumped_bytes = report.pumped_bytes.saturating_add(pumped.bytes);
        forwarders.push(forwarder);
    }
    peak = peak.max(rss(&pids)?);
    finish(&mut report, (handles, tasks), (watchers, viewers, stalled)).await?;
    for forwarder in forwarders {
        forwarder.join();
    }
    report.history_disk_bytes = disk_bytes(&history_root)?;
    std::fs::remove_dir_all(&history_root)?;
    report.admissions = samples.admission.len();
    report.controls = samples.control.len();
    report.rejected = samples.rejected;
    report.admission_p50_ms = clients::quantile_ms(&mut samples.admission, 500);
    report.admission_p99_ms = clients::quantile_ms(&mut samples.admission, 990);
    report.admission_max_ms = clients::max_ms(&samples.admission);
    report.control_max_ms = clients::max_ms(&samples.control);
    report.rss_peak_bytes = peak;
    report.rss_after_bytes = rss(&[])?;
    Ok(report)
}

type Listeners = (
    JoinSet<(Vec<Duration>, u64)>,
    JoinSet<(u64, u64)>,
    Vec<Receiver<OutputEvent>>,
);

async fn finish(
    report: &mut LoadReport,
    (handles, tasks): (Vec<TerminalHandle>, Vec<ActorTask>),
    (mut watchers, mut viewers, stalled): Listeners,
) -> Result<(), TuningError> {
    for handle in &handles {
        handle.stop()?;
    }
    for task in tasks {
        let actor = task.join().await?;
        report.parsed_bytes = report.parsed_bytes.saturating_add(actor.parsed_bytes);
        report.parse_turns = report.parse_turns.saturating_add(actor.parse_turns);
        report.output_waits = report.output_waits.saturating_add(actor.output_waits);
        report.input_unsent_entries = report.input_unsent_entries.saturating_add(actor.unsent);
        match actor.history {
            Some(history) => {
                report.history_recorded_bytes =
                    report.history_recorded_bytes.saturating_add(history.end);
                report.history_evicted_segments = report
                    .history_evicted_segments
                    .saturating_add(history.evicted_segments);
                if history.fault.is_some() {
                    report.history_faults = report.history_faults.saturating_add(1);
                }
            }
            None => report.history_faults = report.history_faults.saturating_add(1),
        }
        let turn = actor.max_turn.as_secs_f64() * 1e3;
        report.max_turn_ms = report.max_turn_ms.max(turn);
    }
    report.stalled_viewers = stalled.len();
    for viewer in stalled {
        let (_, lost) = clients::drain_stalled(viewer);
        report.stalled_events_lost = report.stalled_events_lost.saturating_add(lost);
    }
    drop(handles);
    let mut events = Vec::new();
    while let Some(Ok((latency, lost))) = watchers.join_next().await {
        events.extend(latency);
        report.events_lost = report.events_lost.saturating_add(lost);
    }
    while let Some(Ok((bytes, lost))) = viewers.join_next().await {
        report.viewer_bytes = report.viewer_bytes.saturating_add(bytes);
        report.viewer_events_lost = report.viewer_events_lost.saturating_add(lost);
    }
    report.events = events.len();
    report.event_max_ms = clients::max_ms(&events);
    Ok(())
}
