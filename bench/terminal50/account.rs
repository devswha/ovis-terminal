//! The report of a settled run: every account turned into distributions, the realized
//! shape beside the requested one, and the verdict reached from exactly those values.

use serde::Serialize;

use super::delivery::ClientOutcome;
use super::host::{self, Binary};
use super::plan::{Plan, Profile};
use super::report::{ClientLag, Footprint, Phases, Report, Verdict};
use super::settle::Load;
use super::stats;
use super::verdict::{self, Budgets, Observed, Realized};

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

const fn requested(plan: &Plan) -> Realized {
    Realized {
        terminals: plan.shape.terminals,
        viewers: plan.shape.terminals.saturating_mul(plan.viewers),
        clients: plan.shape.clients,
        lanes: plan.shape.inflight,
    }
}

fn realized(load: &Load) -> Realized {
    let settled = &load.settled;
    Realized {
        terminals: settled.live_terminals,
        viewers: settled
            .viewers
            .iter()
            .filter(|viewer| !viewer.reads || viewer.bytes > 0)
            .count(),
        clients: settled
            .clients
            .iter()
            .filter(|client| client.reads && !client.written.measured.is_empty())
            .count(),
        lanes: load.lanes,
    }
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

fn observed(plan: &Plan, load: &Load) -> Observed {
    let mut observed = Observed {
        admission_warmup: load.samples.warmup.clone(),
        admission: load.samples.admission.clone(),
        wanted_samples: load.wanted,
        rss_peak_bytes: load.rss_peak,
        disk_bytes: load.disk,
        requested: requested(plan),
        realized: realized(load),
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
    let observed = observed(plan, load);
    let budgets = Budgets {
        rss_limit_bytes: plan.rss_limit_bytes,
        disk_limit_bytes: plan.disk_limit_bytes,
    };
    let violations = verdict::judge(plan.profile, &observed, budgets);
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
        requested: observed.requested,
        realized: observed.realized,
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
