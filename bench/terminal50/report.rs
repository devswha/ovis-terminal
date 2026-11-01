//! The report of one run: every distribution, account and footprint, and the verdict
//! [`super::verdict`] reached from them.

use std::collections::BTreeMap;

use serde::Serialize;

use super::host::{Binary, Build, Host};
use super::plan::{Fixture, Profile, Shape};
use super::stats::{self, Distribution};
use super::verdict::{Budgets, Realized, Violation};

/// One distribution with the raw-sample count it was built from.
#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct Phases {
    pub(super) warmup: Distribution,
    pub(super) measured: Distribution,
}

impl Phases {
    pub(super) fn of(warmup: &[std::time::Duration], measured: &[std::time::Duration]) -> Self {
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
    pub(super) requested: Realized,
    pub(super) realized: Realized,
    pub(super) clients: Vec<ClientLag>,
    pub(super) viewers: Vec<ViewerLag>,
    pub(super) footprint: Footprint,
    pub(super) budgets: Budgets,
    pub(super) verdict: Verdict,
}
