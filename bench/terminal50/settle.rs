//! Stopping a run in dependency order and turning every account into the report.
//!
//! Actors stop first, then their programs and stdin forwarders; once every event sender
//! is gone the clients' forwarders end, a non-reader's unread end is hung up, and each
//! writer drains within its bound. Nothing recorded is dropped on the way.

use std::time::Duration;

use ovis_terminal_server::streams::scheduler::OutputEvent;
use serde::Serialize;
use tokio::sync::broadcast::error::TryRecvError;

use super::delivery::{Client, ClientOutcome};
use super::error::Terminal50Error;
use super::fleet::{self, Fleet};
use super::host::{self, Binary};
use super::lanes::LaneSamples;
use super::plan::{Plan, Profile};
use super::report::{
    self, Budgets, ClientLag, Footprint, Observed, Phases, Report, Verdict, ViewerLag,
};
use super::stats;

#[derive(Debug, Default)]
pub(super) struct Settled {
    pub(super) clients: Vec<ClientOutcome>,
    pub(super) viewers: Vec<ViewerLag>,
    pub(super) pumped: u64,
    pub(super) parsed: u64,
    pub(super) max_turn: Duration,
    pub(super) committed: u64,
}

#[derive(Debug)]
pub(super) struct Load {
    pub(super) samples: LaneSamples,
    pub(super) wanted: usize,
    pub(super) rss_before: u64,
    pub(super) rss_peak: u64,
    pub(super) disk: u64,
    pub(super) cpu: (Duration, Duration),
    pub(super) wall: Duration,
    pub(super) settled: Settled,
}

/// Every raw sample in microseconds, in recording order.
#[derive(Debug, Serialize)]
pub(super) struct Raw {
    pub(super) unit: &'static str,
    pub(super) admission_warmup: Vec<u64>,
    pub(super) admission: Vec<u64>,
    pub(super) control: Vec<u64>,
    pub(super) event_warmup: Vec<u64>,
    pub(super) event: Vec<u64>,
    pub(super) non_reader_event: Vec<u64>,
}

fn stalled_viewer(
    (terminal, mut output): (usize, tokio::sync::broadcast::Receiver<OutputEvent>),
) -> ViewerLag {
    let mut lag = ViewerLag {
        terminal,
        viewer: 0,
        reads: false,
        bytes: 0,
        events_lost: 0,
        events_held: 0,
    };
    loop {
        match output.try_recv() {
            Ok(event) => {
                lag.events_held = lag.events_held.saturating_add(1);
                if let OutputEvent::Chunk { bytes, .. } = event {
                    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    lag.bytes = lag.bytes.saturating_add(len);
                }
            }
            Err(TryRecvError::Lagged(missed)) => {
                lag.events_lost = lag.events_lost.saturating_add(missed);
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => return lag,
        }
    }
}

/// Stops `fleet` and settles every client and viewer.
pub(super) async fn settle(
    plan: &Plan,
    fleet: Fleet,
    clients: Vec<Client>,
) -> Result<Settled, Terminal50Error> {
    let Fleet {
        handles,
        tasks,
        programs,
        mut viewers,
        stalled,
        ..
    } = fleet;
    let mut settled = Settled::default();
    for handle in &handles {
        handle.stop()?;
    }
    let mut forwarders = Vec::new();
    for program in programs {
        let (pumped, forward) = fleet::stop(program).await?;
        settled.pumped = settled.pumped.saturating_add(pumped);
        forwarders.extend(forward);
    }
    for task in tasks {
        let actor = task.join().await?;
        settled.parsed = settled.parsed.saturating_add(actor.parsed_bytes);
        settled.max_turn = settled.max_turn.max(actor.max_turn);
        let committed = actor
            .admitted
            .saturating_add(actor.withdrawn)
            .saturating_add(actor.resized);
        settled.committed = settled.committed.saturating_add(committed);
    }
    for forward in forwarders {
        tokio::task::spawn_blocking(move || forward.join())
            .await
            .map_err(std::io::Error::other)?
            .map_err(|_panicked| std::io::Error::other("a stdin forwarder panicked"))?;
    }
    drop(handles);
    for mut client in clients {
        client.hang_up_unread();
        settled.clients.push(client.finish(plan.drain).await?);
    }
    while let Some(viewed) = viewers.join_next().await {
        let viewed = viewed.map_err(std::io::Error::other)?;
        settled.viewers.push(ViewerLag {
            terminal: viewed.terminal,
            viewer: viewed.viewer,
            reads: true,
            bytes: viewed.bytes,
            events_lost: viewed.lost,
            events_held: 0,
        });
    }
    settled
        .viewers
        .extend(stalled.into_iter().map(stalled_viewer));
    settled
        .viewers
        .sort_by_key(|lag| (lag.terminal, lag.viewer));
    Ok(settled)
}

fn client_lag(outcome: &ClientOutcome) -> ClientLag {
    ClientLag {
        client: outcome.index,
        reads: outcome.reads,
        delivered: outcome
            .written
            .warmup
            .len()
            .saturating_add(outcome.written.measured.len()),
        lost: outcome.lost,
        undelivered: outcome.written.undelivered,
        deepest_queue: outcome.deepest_queue,
        write_failed: outcome.written.write_failed,
        bytes_read: outcome.bytes_read,
        latency: Phases::of(&outcome.written.warmup, &outcome.written.measured),
    }
}

fn observed(load: &Load) -> Observed {
    let mut observed = Observed {
        admission_warmup: load.samples.warmup.clone(),
        admission: load.samples.admission.clone(),
        wanted_samples: load.wanted,
        rss_peak_bytes: load.rss_peak,
        disk_bytes: load.disk,
        ..Observed::default()
    };
    for outcome in load.settled.clients.iter().filter(|outcome| outcome.reads) {
        observed.event_warmup.extend(&outcome.written.warmup);
        observed.event.extend(&outcome.written.measured);
        let missing = outcome.lost.saturating_add(outcome.written.undelivered);
        observed.events_missing = observed.events_missing.saturating_add(missing);
    }
    observed
}

/// The report and raw samples of a settled run.
pub(super) fn report(plan: &Plan, binary: Binary, load: &Load) -> (Report, Raw) {
    let observed = observed(load);
    let budgets = Budgets {
        rss_limit_bytes: plan.rss_limit_bytes,
        disk_limit_bytes: plan.disk_limit_bytes,
    };
    let violations = report::judge(plan.profile, &observed, budgets);
    let non_reader: Vec<std::time::Duration> = load
        .settled
        .clients
        .iter()
        .filter(|outcome| !outcome.reads)
        .flat_map(|outcome| {
            outcome
                .written
                .warmup
                .iter()
                .chain(&outcome.written.measured)
        })
        .copied()
        .collect();
    let raw = Raw {
        unit: "microseconds",
        admission_warmup: stats::micros(&observed.admission_warmup),
        admission: stats::micros(&observed.admission),
        control: stats::micros(&load.samples.control),
        event_warmup: stats::micros(&observed.event_warmup),
        event: stats::micros(&observed.event),
        non_reader_event: stats::micros(&non_reader),
    };
    let report = Report {
        format: "ovis-terminal50/1",
        label: plan.label(),
        judged_on: match plan.profile {
            Profile::Ideal { .. } => "every admission and event sample, warmup included",
            Profile::Capacity { .. } => "p99 of the measured samples",
        },
        surface: SURFACE,
        budget: plan.profile,
        shape: plan.shape,
        viewers_per_terminal: plan.viewers,
        warmup_samples: plan.warmup,
        samples_wanted: plan.samples,
        fixture: plan.fixture,
        host: host::host(plan.workers),
        build: host::build(),
        binary,
        admission: Phases::of(&observed.admission_warmup, &observed.admission),
        admitted: load.samples.admitted,
        rejected: load.samples.rejected.clone(),
        control: stats::summarize(&load.samples.control),
        event: Phases::of(&observed.event_warmup, &observed.event),
        events_committed: load.settled.committed,
        clients: load.settled.clients.iter().map(client_lag).collect(),
        viewers: load.settled.viewers.clone(),
        footprint: Footprint {
            rss_before_bytes: load.rss_before,
            rss_peak_bytes: load.rss_peak,
            disk_bytes: load.disk,
            runtime_cpu_ms: stats::ms(load.cpu.0),
            programs_cpu_ms: stats::ms(load.cpu.1),
            wall_ms: stats::ms(load.wall),
            pumped_bytes: load.settled.pumped,
            parsed_bytes: load.settled.parsed,
            max_parse_turn_ms: stats::ms(load.settled.max_turn),
        },
        budgets,
        verdict: Verdict {
            passed: violations.is_empty(),
            violations,
        },
    };
    (report, raw)
}

/// What the samples measure, stated in every report.
const SURFACE: &str = "ovis-terminal-server scheduler actors (one per terminal, shared session budgets, disk history) fed by real PTY programs; admission times reserve+admit to receipt, events time actor commit to the line accepted by each client's Unix socket. The binary under test is hashed and versioned; it routes no input.* or events.* call to these actors before todo 31 assembles it.";
