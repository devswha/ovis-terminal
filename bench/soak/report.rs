//! The evidence report: plan, host, binary, every measured fact, the raw companions'
//! paths and the verdict. An errored run still reports, and never as a pass.

use std::time::Duration;

use serde_json::{Value, json};

use super::drive::Run;
use super::error::SoakError;
use super::fixture::FixtureOutcome;
use super::host::{Binary, host};
use super::verdict::{Measured, Probe, Violation, growth_allowance, judge};

/// What the run produced besides the run state itself.
#[derive(Debug, Default)]
pub(super) struct Outcome {
    pub(super) elapsed: Duration,
    pub(super) probe: Option<Probe>,
    pub(super) fixture: Option<FixtureOutcome>,
    pub(super) error: Option<SoakError>,
    pub(super) teardown: Option<SoakError>,
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn error_json(error: Option<&SoakError>) -> Value {
    error.map_or(
        Value::Null,
        |error| json!({"kind": error.kind(), "message": error.to_string(), "hint": error.hint()}),
    )
}

fn violations(run: &Run<'_>, outcome: &Outcome) -> (Vec<Violation>, Value) {
    let plan = run.plan;
    let baseline = run
        .samples
        .median((plan.warmup, plan.warmup.saturating_add(plan.window)));
    let last = run
        .samples
        .median((plan.duration.saturating_sub(plan.window), plan.duration));
    let measured = Measured {
        duration: plan.duration,
        elapsed: outcome.elapsed,
        requested: plan.counts.clone(),
        done: run.done.clone(),
        tally: &run.tally,
        continuity: run.continuity,
        limits: plan.limits,
        rss_peak: run.samples.peak_runtime,
        rss_baseline: baseline,
        rss_final: last,
        disk_peak: run.samples.peak_disk,
        planned_exits: run.planned_exits.clone(),
        probe: plan
            .holder_loss_probe
            .then(|| outcome.probe.clone().unwrap_or_default()),
    };
    let rss = json!({
        "peak_bytes": run.samples.peak_runtime,
        "baseline_median_bytes": baseline,
        "final_median_bytes": last,
        "growth_bytes": baseline.zip(last).map(|(b, l)| l.saturating_sub(b)),
        "growth_allowance_bytes": baseline.map(|b| growth_allowance(&plan.limits, b)),
    });
    let mut found = if plan.fixture.is_some() {
        Vec::new()
    } else {
        judge(&measured)
    };
    if let Some(fixture) = &outcome.fixture
        && !fixture.passed()
    {
        found.push(Violation {
            rule: "fixture_expectation",
            observed: "exact recovery, no gap, a known outcome or a resend".to_owned(),
            limit: "screen_exact false, a gap, outcome_unknown, no resend".to_owned(),
        });
    }
    for (rule, error) in [
        ("run_error", &outcome.error),
        ("teardown_error", &outcome.teardown),
    ] {
        if let Some(error) = error {
            found.push(Violation {
                rule,
                observed: error.kind().to_owned(),
                limit: "none".to_owned(),
            });
        }
    }
    (found, rss)
}

/// The report of `run` and how many rules it broke.
pub(super) fn report(run: &Run<'_>, (binary, outcome): (&Binary, &Outcome)) -> (Value, usize) {
    let plan = run.plan;
    let (found, rss) = violations(run, outcome);
    let passed = found.is_empty();
    let found_count = found.len();
    let value = json!({
        "format": "ovis-soak/1",
        "label": plan.label,
        "counts_as_acceptance": plan.label == "continuous",
        "host": host(),
        "binary": binary,
        "plan": {
            "terminals": plan.terminals,
            "duration_ms": millis(plan.duration),
            "warmup_ms": millis(plan.warmup),
            "window_ms": millis(plan.window),
            "bytes_per_terminal_per_second": plan.bytes_per_terminal,
            "checkpoint_every_ms": millis(plan.checkpoint_every),
            "history_lines": plan.history_lines,
            "requested": plan.counts,
            "schedule": plan.disruptions,
            "holder_loss_probe": plan.holder_loss_probe,
            "limits": plan.limits,
            "fixture": plan.fixture,
        },
        "measured": {
            "elapsed_ms": millis(outcome.elapsed),
            "done": run.done,
            "control_boots": run.boot,
            "downtimes_ms": run.downtimes_ms,
            "planned_restart_exit_codes": run.planned_exits,
            "rss": rss,
            "disk_peak_bytes": run.samples.peak_disk,
            "samples": run.samples.count,
            "continuity": run.continuity,
            "tally": run.tally,
        },
        "companions": {
            "samples": run.samples.path,
            "checkpoint": run.samples.checkpoint,
            "logs": run.env.logs,
        },
        "probe": outcome.probe,
        "fixture": outcome.fixture.as_ref().map(|fixture| json!({
            "outcome": fixture,
            "screen_exact": fixture.screen_exact,
            "exact_recovery_claimed": fixture.screen_exact == Some(true),
            "auto_resend": fixture.resent,
            "passed": fixture.passed(),
        })),
        "holders": run.started_holders,
        "cli": run.cli,
        "error": error_json(outcome.error.as_ref()),
        "teardown_error": error_json(outcome.teardown.as_ref()),
        "verdict": { "passed": passed, "violations": found },
    });
    (value, found_count)
}
