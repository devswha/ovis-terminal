//! Behaviour of the verdict, the argument rules and real in-process runs.
//!
//! The runs assert only what host speed cannot change: a writer delayed 150 ms can never
//! deliver within 100 ms, and every committed event is delivered, lost or undelivered.

use std::time::Duration;

use clap::Parser as _;

use super::cli::Cli;
use super::error::Terminal50Error;
use super::measure::{parse_clock, stat_cpu};
use super::plan::{Fixture, Plan, Profile};
use super::stats::summarize;
use super::verdict::{Budgets, Observed, judge};

mod bound;
mod runs;
mod shape;

const BUDGETS: Budgets = Budgets {
    rss_limit_bytes: 1 << 30,
    disk_limit_bytes: 1 << 32,
};
const IDEAL: Profile = Profile::Ideal {
    admission_max_ms: 100,
    event_max_ms: 100,
};
const CAPACITY: Profile = Profile::Capacity {
    admission_p99_ms: 100,
    event_p99_ms: 100,
};

fn ms(values: &[u64]) -> Vec<Duration> {
    values.iter().copied().map(Duration::from_millis).collect()
}

fn fast(count: usize) -> Vec<Duration> {
    vec![Duration::from_millis(5); count]
}

fn checks(profile: Profile, observed: &Observed) -> Vec<&'static str> {
    judge(profile, observed, BUDGETS)
        .into_iter()
        .map(|violation| violation.check)
        .collect()
}

fn healthy() -> Observed {
    Observed {
        admission: fast(1000),
        event: fast(1000),
        wanted_samples: 1000,
        ..Observed::default()
    }
}

#[test]
fn nearest_rank_summary_counts_every_sample_and_keeps_the_slowest() {
    let samples = ms(&(1..=100).rev().collect::<Vec<u64>>());
    let summary = summarize(&samples);
    assert_eq!(summary.count, 100);
    assert_eq!(
        (
            summary.p50_ms,
            summary.p95_ms,
            summary.p99_ms,
            summary.max_ms
        ),
        (50.0, 95.0, 99.0, 100.0)
    );
}

#[test]
fn one_slow_sample_fails_the_ideal_profile_but_not_the_capacity_p99() {
    let mut observed = healthy();
    observed.admission.push(Duration::from_millis(150));
    observed.event.push(Duration::from_millis(101));
    assert_eq!(
        checks(IDEAL, &observed),
        ["admission_max_ms", "event_max_ms"]
    );
    assert_eq!(checks(CAPACITY, &observed), Vec::<&str>::new());
    assert_eq!(checks(IDEAL, &healthy()), Vec::<&str>::new());
}

#[test]
fn the_ideal_profile_judges_warmup_samples_too() {
    let mut observed = healthy();
    observed.admission_warmup = ms(&[5, 400]);
    observed.wanted_samples = 1002;
    assert_eq!(checks(IDEAL, &observed), ["admission_max_ms"]);
    assert_eq!(checks(CAPACITY, &observed), Vec::<&str>::new());
}

#[test]
fn a_p99_over_budget_fails_the_capacity_profile() {
    let mut observed = healthy();
    observed.event.extend(ms(&[150; 20]));
    assert_eq!(checks(CAPACITY, &observed), ["event_p99_ms"]);
}

#[test]
fn missing_events_short_runs_and_footprints_fail_both_profiles() {
    let mut observed = healthy();
    observed.events_missing = 1;
    observed.wanted_samples = 1001;
    observed.rss_peak_bytes = BUDGETS.rss_limit_bytes + 1;
    observed.disk_bytes = BUDGETS.disk_limit_bytes + 1;
    let expected = [
        "admission_samples",
        "writable_events_missing",
        "rss_peak_bytes",
        "disk_bytes",
    ];
    assert_eq!(checks(IDEAL, &observed), expected);
    assert_eq!(checks(CAPACITY, &observed), expected);
    observed = healthy();
    observed.event.clear();
    assert_eq!(checks(CAPACITY, &observed), ["measured_event_samples"]);
}

fn plan_of(flags: &[&str]) -> Result<Plan, Terminal50Error> {
    let command_line = ["terminal50", "--binary", "ovis-terminal"]
        .iter()
        .chain(flags);
    Cli::try_parse_from(command_line)
        .map_err(|_| Terminal50Error::Usage { reason: "clap" })?
        .plan("terminal50".into())
        .map(|(plan, _)| plan)
}

const SHAPE16: [&str; 9] = [
    "--isolated",
    "--terminals=16",
    "--clients=8",
    "--inflight=16",
    "--output-bytes-per-second=1048576",
    "--samples=10000",
    "--admission-max-ms=100",
    "--event-max-ms=100",
    "--warmup-samples=1000",
];

#[test]
fn the_max_bounds_judge_only_the_exact_ideal_shape() {
    let ideal = plan_of(&SHAPE16).unwrap();
    assert_eq!((ideal.profile, ideal.label()), (IDEAL, "ideal-16pty"));
    assert_eq!(ideal.viewers, 8);
    let mut fifty = SHAPE16;
    fifty[1] = "--terminals=50";
    assert_eq!(plan_of(&fifty).unwrap_err().kind(), "usage");
    fifty[6] = "--admission-p99-ms=100";
    fifty[7] = "--event-p99-ms=100";
    let capacity = plan_of(&fifty).unwrap();
    assert_eq!(
        (capacity.profile, capacity.label()),
        (CAPACITY, "capacity-release-target")
    );
    assert_eq!(
        capacity.shape.terminals.saturating_mul(capacity.viewers),
        400
    );
    fifty[1] = "--terminals=49";
    assert_eq!(plan_of(&fifty).unwrap_err().kind(), "usage");
}

#[test]
fn mixed_budgets_a_shared_root_and_unknown_fixtures_are_refused() {
    let mut mixed = SHAPE16;
    mixed[7] = "--event-p99-ms=100";
    assert_eq!(plan_of(&mixed).unwrap_err().kind(), "usage");
    let mut shared = SHAPE16;
    shared[0] = "--viewers-per-terminal=8";
    assert_eq!(plan_of(&shared).unwrap_err().kind(), "isolation_required");
    let inject = |payload: &str| {
        let mut args = SHAPE16.to_vec();
        let flag = format!("--inject={payload}");
        args.push(&flag);
        plan_of(&args).map(|plan| plan.fixture)
    };
    let delayed = inject(r#"{"fixture":"control-writer-delay","delay_ms":150}"#).unwrap();
    assert_eq!(delayed, Some(Fixture::ControlWriterDelay { delay_ms: 150 }));
    let admission = inject(r#"{"fixture":"admission-delay","delay_ms":150}"#).unwrap();
    assert_eq!(admission, Some(Fixture::AdmissionDelay { delay_ms: 150 }));
    for refused in [
        r#"{"fixture":"other","delay_ms":150}"#,
        r#"{"fixture":"control-writer-delay","delay_ms":150,"filter":true}"#,
        r#"{"fixture":"control-writer-delay","delay_ms":"150"}"#,
    ] {
        assert_eq!(
            inject(refused).unwrap_err().kind(),
            "invalid_inject",
            "{refused}"
        );
    }
}

#[test]
fn ps_cpu_clocks_parse_with_hours_minutes_and_fractions() {
    assert_eq!(parse_clock("1:02.50"), Some(Duration::from_millis(62_500)));
    assert_eq!(parse_clock("1:00:00"), Some(Duration::from_secs(3600)));
    assert_eq!(parse_clock("0:-1"), None);
    assert_eq!(parse_clock("busy"), None);
}

#[test]
fn proc_stat_cpu_sums_user_and_system_ticks_after_the_command_name() {
    let stat = "4242 (tokio (rt) x) S 1 4242 4242 0 -1 4194560 10 0 0 0 250 125 7 9 20 0";
    assert_eq!(stat_cpu(stat, 100), Some(Duration::from_millis(3750)));
    assert_eq!(stat_cpu("4242 (no fields)", 100), None);
}
