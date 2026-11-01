//! The verdict: every rule a run is judged by, from measured facts only. A run that
//! stopped early, skipped a disruption or lost a program fails; nothing is filtered.

use std::time::Duration;

use serde::Serialize;

use super::account::Tally;
use super::observer::Continuity;
use super::plan::{Kind, Limits};

/// What the internal holder-loss probe found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(super) struct Probe {
    /// The dedicated terminal's survival after its holder was killed.
    pub(super) dedicated: String,
    pub(super) shared_survived: u64,
    pub(super) shared_expected: u64,
    pub(super) shared_identities_unchanged: bool,
}

/// Everything a verdict needs.
#[derive(Debug)]
pub(super) struct Measured<'a> {
    pub(super) duration: Duration,
    pub(super) elapsed: Duration,
    pub(super) requested: Vec<(Kind, usize)>,
    pub(super) done: Vec<(Kind, usize)>,
    pub(super) tally: &'a Tally,
    pub(super) continuity: Continuity,
    pub(super) limits: Limits,
    pub(super) rss_peak: u64,
    pub(super) rss_baseline: Option<u64>,
    pub(super) rss_final: Option<u64>,
    pub(super) disk_peak: u64,
    pub(super) planned_exits: Vec<Option<i32>>,
    /// `None` when the probe was not requested.
    pub(super) probe: Option<Probe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Violation {
    pub(super) rule: &'static str,
    pub(super) observed: String,
    pub(super) limit: String,
}

fn rule(rule: &'static str, observed: &impl ToString, limit: &impl ToString) -> Violation {
    Violation {
        rule,
        observed: observed.to_string(),
        limit: limit.to_string(),
    }
}

/// Allowed growth over `baseline`: the fixed bytes plus the percentage.
pub(super) fn growth_allowance(limits: &Limits, baseline: u64) -> u64 {
    let share = baseline
        .saturating_mul(limits.growth_percent)
        .checked_div(100)
        .unwrap_or(0);
    limits.growth_bytes.saturating_add(share)
}

fn memory(measured: &Measured<'_>, found: &mut Vec<Violation>) {
    let limits = &measured.limits;
    if measured.rss_peak > limits.rss_bytes {
        found.push(rule("rss_limit", &measured.rss_peak, &limits.rss_bytes));
    }
    match (measured.rss_baseline, measured.rss_final) {
        (Some(baseline), Some(last)) => {
            let allowed = growth_allowance(limits, baseline);
            let grown = last.saturating_sub(baseline);
            if grown > allowed {
                found.push(rule("rss_growth", &grown, &allowed));
            }
        }
        _ => found.push(rule(
            "rss_windows",
            &"a window had no sample",
            &"samples in both windows",
        )),
    }
    if measured.disk_peak > limits.disk_bytes {
        found.push(rule("disk_limit", &measured.disk_peak, &limits.disk_bytes));
    }
}

fn runtime(measured: &Measured<'_>, found: &mut Vec<Violation>) {
    let tally = measured.tally;
    if !tally.losses.is_empty() {
        found.push(rule("unintended_child_loss", &tally.losses.len(), &0));
    }
    if tally.offset_regressions > 0 {
        found.push(rule(
            "output_offset_regression",
            &tally.offset_regressions,
            &0,
        ));
    }
    let unreported: u64 = tally
        .recoveries
        .iter()
        .map(|note| note.unreported_screens)
        .sum();
    if unreported > 0 {
        found.push(rule("recovery_screen_unreported", &unreported, &0));
    }
    if measured.continuity.holes > 0 {
        found.push(rule("sequence_hole", &measured.continuity.holes, &0));
    }
    if tally.max_queued_bytes > measured.limits.queue_bytes {
        found.push(rule(
            "viewer_queue_bound",
            &tally.max_queued_bytes,
            &measured.limits.queue_bytes,
        ));
    }
    if tally.control_max_ms > measured.limits.control_ms || tally.late_pongs > 0 {
        let observed = format!("{} ms max, {} late", tally.control_max_ms, tally.late_pongs);
        found.push(rule(
            "control_unavailable",
            &observed,
            &measured.limits.control_ms,
        ));
    }
    if tally.pongs == 0 {
        found.push(rule("control_unprobed", &0, &"at least one answered ping"));
    }
    if tally.inputs_sent > 0 {
        found.push(rule("unrequested_input", &tally.inputs_sent, &0));
    }
    let failed_exits = measured
        .planned_exits
        .iter()
        .filter(|code| **code != Some(0))
        .count();
    if failed_exits > 0 {
        found.push(rule("planned_restart_exit", &failed_exits, &0));
    }
}

fn probe(measured: &Measured<'_>, found: &mut Vec<Violation>) {
    let Some(probe) = &measured.probe else { return };
    if probe.dedicated != "unknown:holder_unreachable" {
        found.push(rule(
            "dedicated_holder_loss",
            &&probe.dedicated,
            &"unknown:holder_unreachable",
        ));
    }
    if probe.shared_survived != probe.shared_expected || !probe.shared_identities_unchanged {
        found.push(rule(
            "shared_holder_isolation",
            &probe.shared_survived,
            &probe.shared_expected,
        ));
    }
}

/// Every rule `measured` breaks; empty means the run passed.
pub(super) fn judge(measured: &Measured<'_>) -> Vec<Violation> {
    let mut found = Vec::new();
    if measured.elapsed < measured.duration {
        found.push(rule(
            "incomplete_duration",
            &measured.elapsed.as_millis(),
            &measured.duration.as_millis(),
        ));
    }
    for (kind, wanted) in &measured.requested {
        let did = measured
            .done
            .iter()
            .find(|(done, _)| done == kind)
            .map_or(0, |(_, count)| *count);
        if did != *wanted {
            let name = serde_json::to_value(kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned));
            found.push(rule(
                "disruption_count",
                &format!("{} {did}", name.unwrap_or_default()),
                wanted,
            ));
        }
    }
    memory(measured, &mut found);
    runtime(measured, &mut found);
    probe(measured, &mut found);
    found
}
