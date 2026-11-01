//! The report of one run and its verdict.
//!
//! The IDEAL profile fails on any single admission or event sample over its bound,
//! warmup included; the capacity profile is judged on the p99 of the measured samples.
//! Both fail when an event to a writable client was lost or never written, when fewer
//! samples were taken than asked, or when memory or disk exceeds its session budget.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use super::host::{Binary, Build, Host};
use super::plan::{Fixture, Profile, Shape};
use super::stats::{self, Distribution};

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

const fn bound(limit_ms: u64) -> Duration {
    Duration::from_millis(limit_ms)
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
        stats::over(&all, bound(limit_ms)),
    )
}

fn percentile(measured: &[Duration], limit_ms: u64) -> (f64, usize) {
    let p99 = stats::summarize(measured).p99_ms;
    (p99, usize::from(p99 > stats::ms(bound(limit_ms))))
}

fn count(check: &'static str, (limit, observed): (u64, u64), at_most: bool) -> Option<Violation> {
    let broken = if at_most {
        observed > limit
    } else {
        observed < limit
    };
    broken.then(|| Violation {
        check,
        limit: json!(limit),
        observed: json!(observed),
    })
}

/// Every budget `observed` breaks under `profile` and the session `budgets`.
pub(super) fn judge(profile: Profile, observed: &Observed, budgets: Budgets) -> Vec<Violation> {
    let latencies = match profile {
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
    };
    let wanted = u64::try_from(observed.wanted_samples).unwrap_or(u64::MAX);
    let taken = observed
        .admission_warmup
        .len()
        .saturating_add(observed.admission.len());
    let events = u64::try_from(observed.event.len()).unwrap_or(u64::MAX);
    latencies
        .into_iter()
        .chain([
            count(
                "admission_samples",
                (wanted, u64::try_from(taken).unwrap_or(u64::MAX)),
                false,
            ),
            count("measured_event_samples", (1, events), false),
            count(
                "writable_events_missing",
                (0, observed.events_missing),
                true,
            ),
            count(
                "rss_peak_bytes",
                (budgets.rss_limit_bytes, observed.rss_peak_bytes),
                true,
            ),
            count(
                "disk_bytes",
                (budgets.disk_limit_bytes, observed.disk_bytes),
                true,
            ),
        ])
        .flatten()
        .collect()
}

/// One distribution with the raw-sample count it was built from.
#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct Phases {
    pub(super) warmup: Distribution,
    pub(super) measured: Distribution,
}

impl Phases {
    pub(super) fn of(warmup: &[Duration], measured: &[Duration]) -> Self {
        Self {
            warmup: stats::summarize(warmup),
            measured: stats::summarize(measured),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ClientLag {
    pub(super) client: usize,
    pub(super) reads: bool,
    pub(super) delivered: usize,
    pub(super) lost: u64,
    pub(super) undelivered: u64,
    pub(super) deepest_queue: usize,
    pub(super) write_failed: bool,
    pub(super) bytes_read: u64,
    pub(super) latency: Phases,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ViewerLag {
    pub(super) terminal: usize,
    pub(super) viewer: usize,
    pub(super) reads: bool,
    pub(super) bytes: u64,
    pub(super) events_lost: u64,
    pub(super) events_held: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Footprint {
    pub(super) rss_before_bytes: u64,
    pub(super) rss_peak_bytes: u64,
    pub(super) disk_bytes: u64,
    pub(super) runtime_cpu_ms: f64,
    pub(super) programs_cpu_ms: f64,
    pub(super) wall_ms: f64,
    pub(super) pumped_bytes: u64,
    pub(super) parsed_bytes: u64,
    pub(super) max_parse_turn_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Verdict {
    pub(super) passed: bool,
    pub(super) violations: Vec<Violation>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Report {
    pub(super) format: &'static str,
    pub(super) label: &'static str,
    pub(super) judged_on: &'static str,
    pub(super) surface: &'static str,
    pub(super) budget: Profile,
    pub(super) shape: Shape,
    pub(super) viewers_per_terminal: usize,
    pub(super) warmup_samples: usize,
    pub(super) samples_wanted: usize,
    pub(super) fixture: Option<Fixture>,
    pub(super) host: Host,
    pub(super) build: Build,
    pub(super) binary: Binary,
    pub(super) admission: Phases,
    pub(super) admitted: u64,
    pub(super) rejected: BTreeMap<&'static str, u64>,
    pub(super) control: Distribution,
    pub(super) event: Phases,
    pub(super) events_committed: u64,
    pub(super) clients: Vec<ClientLag>,
    pub(super) viewers: Vec<ViewerLag>,
    pub(super) footprint: Footprint,
    pub(super) budgets: Budgets,
    pub(super) verdict: Verdict,
}
