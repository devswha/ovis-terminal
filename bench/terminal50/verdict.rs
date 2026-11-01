//! The verdict of one run, read only from what the run measured.
//!
//! The IDEAL profile fails on any single admission or event sample over its bound,
//! warmup included; the capacity profile is judged on the p99 of the measured samples.
//! Both fail when an event to a writable client was lost or never written, when fewer
//! samples were taken than asked, when memory or disk exceeds its session budget, and
//! when fewer terminals, viewers, clients or request lanes actually ran than requested.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use super::plan::Profile;
use super::stats;

/// What a run put to work: requested from the plan, or realized by the run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(super) struct Realized {
    /// Terminals whose actor parsed program output.
    pub(super) terminals: usize,
    /// Output viewers alive at the end; a reading one counts once it received output.
    pub(super) viewers: usize,
    /// Writable control-event clients that delivered at least one event.
    pub(super) clients: usize,
    /// Request lanes that took at least one sample.
    pub(super) lanes: usize,
}

/// Every sample and count the verdict reads.
#[derive(Debug, Default)]
pub(super) struct Observed {
    pub(super) admission_warmup: Vec<Duration>,
    pub(super) admission: Vec<Duration>,
    pub(super) event_warmup: Vec<Duration>,
    pub(super) event: Vec<Duration>,
    pub(super) events_missing: u64,
    pub(super) wanted_samples: usize,
    pub(super) rss_peak_bytes: u64,
    pub(super) disk_bytes: u64,
    pub(super) requested: Realized,
    pub(super) realized: Realized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Budgets {
    pub(super) rss_limit_bytes: u64,
    pub(super) disk_limit_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct Violation {
    pub(super) check: &'static str,
    pub(super) limit: Value,
    pub(super) observed: Value,
}

fn latency(
    check: &'static str,
    limit_ms: u64,
    (observed_ms, over): (f64, usize),
) -> Option<Violation> {
    (over > 0).then(|| Violation {
        check,
        limit: json!(limit_ms),
        observed: json!({"ms": observed_ms, "samples_over": over}),
    })
}

fn every_sample(warmup: &[Duration], measured: &[Duration], limit_ms: u64) -> (f64, usize) {
    let all: Vec<Duration> = warmup.iter().chain(measured).copied().collect();
    (
        stats::summarize(&all).max_ms,
        stats::over(&all, Duration::from_millis(limit_ms)),
    )
}

fn percentile(measured: &[Duration], limit_ms: u64) -> (f64, usize) {
    let p99 = stats::summarize(measured).p99_ms;
    (
        p99,
        usize::from(p99 > stats::ms(Duration::from_millis(limit_ms))),
    )
}

fn at_most(check: &'static str, (limit, observed): (u64, u64)) -> Option<Violation> {
    (observed > limit).then(|| Violation {
        check,
        limit: json!(limit),
        observed: json!(observed),
    })
}

fn at_least(check: &'static str, (limit, observed): (usize, usize)) -> Option<Violation> {
    (observed < limit).then(|| Violation {
        check,
        limit: json!(limit),
        observed: json!(observed),
    })
}

fn latencies(profile: Profile, observed: &Observed) -> [Option<Violation>; 2] {
    match profile {
        Profile::Ideal {
            admission_max_ms,
            event_max_ms,
        } => [
            latency(
                "admission_max_ms",
                admission_max_ms,
                every_sample(
                    &observed.admission_warmup,
                    &observed.admission,
                    admission_max_ms,
                ),
            ),
            latency(
                "event_max_ms",
                event_max_ms,
                every_sample(&observed.event_warmup, &observed.event, event_max_ms),
            ),
        ],
        Profile::Capacity {
            admission_p99_ms,
            event_p99_ms,
        } => [
            latency(
                "admission_p99_ms",
                admission_p99_ms,
                percentile(&observed.admission, admission_p99_ms),
            ),
            latency(
                "event_p99_ms",
                event_p99_ms,
                percentile(&observed.event, event_p99_ms),
            ),
        ],
    }
}

fn shape(observed: &Observed) -> [Option<Violation>; 4] {
    let (requested, realized) = (observed.requested, observed.realized);
    [
        at_least(
            "terminals_realized",
            (requested.terminals, realized.terminals),
        ),
        at_least("viewers_realized", (requested.viewers, realized.viewers)),
        at_least("clients_realized", (requested.clients, realized.clients)),
        at_least("lanes_realized", (requested.lanes, realized.lanes)),
    ]
}

/// Every budget `observed` breaks under `profile` and the session `budgets`.
pub(super) fn judge(profile: Profile, observed: &Observed, budgets: Budgets) -> Vec<Violation> {
    let taken = observed
        .admission_warmup
        .len()
        .saturating_add(observed.admission.len());
    let counts = [
        at_least("admission_samples", (observed.wanted_samples, taken)),
        at_least("measured_event_samples", (1, observed.event.len())),
        at_most("writable_events_missing", (0, observed.events_missing)),
        at_most(
            "rss_peak_bytes",
            (budgets.rss_limit_bytes, observed.rss_peak_bytes),
        ),
        at_most(
            "disk_bytes",
            (budgets.disk_limit_bytes, observed.disk_bytes),
        ),
    ];
    latencies(profile, observed)
        .into_iter()
        .chain(counts)
        .chain(shape(observed))
        .flatten()
        .collect()
}
