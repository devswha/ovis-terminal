//! The verdict of a real in-process run is bound to what that run measured: a slow
//! admission, a footprint over its limit and a fleet smaller than requested each fail it,
//! and the reported values are the ones the verdict judged.

use std::time::Duration;

use serde_json::Value;

use super::super::error::Terminal50Error;
use super::super::plan::Fixture;
use super::super::{drive, settle};
use super::runs::{binary, in_process};

fn observed_ms(report: &super::super::report::Report, check: &str) -> f64 {
    let violation = report
        .verdict
        .violations
        .iter()
        .find(|violation| violation.check == check)
        .unwrap_or_else(|| panic!("no {check} in {:?}", report.verdict));
    violation
        .observed
        .get("ms")
        .and_then(Value::as_f64)
        .unwrap()
}

#[test]
fn an_admission_delay_breaks_the_admission_budget_through_the_run() {
    let plan = in_process("admission", Some(Fixture::AdmissionDelay { delay_ms: 150 }));
    let (report, raw) = drive::run(&plan, binary()).unwrap();
    std::fs::remove_dir_all(&plan.root).unwrap();
    let judged = observed_ms(&report, "admission_max_ms");
    assert!(judged >= 150.0, "{judged}");
    assert!((judged - report.admission.measured.max_ms).abs() < f64::EPSILON);
    assert!(
        raw.admission.len() >= plan.samples,
        "{}",
        raw.admission.len()
    );
    assert!(raw.admission.iter().all(|us| *us >= 150_000), "{raw:?}");
}

#[test]
fn footprint_limits_below_the_measured_footprint_fail_the_run() {
    let mut plan = in_process("footprint", None);
    plan.rss_limit_bytes = 1;
    plan.disk_limit_bytes = 1;
    let (report, _) = drive::run(&plan, binary()).unwrap();
    std::fs::remove_dir_all(&plan.root).unwrap();
    let observed = |check: &str| {
        report
            .verdict
            .violations
            .iter()
            .find(|violation| violation.check == check)
            .and_then(|violation| violation.observed.as_u64())
    };
    assert!(report.footprint.rss_peak_bytes > 1);
    assert!(report.footprint.disk_bytes > 1);
    assert_eq!(
        observed("rss_peak_bytes"),
        Some(report.footprint.rss_peak_bytes)
    );
    assert_eq!(observed("disk_bytes"), Some(report.footprint.disk_bytes));
}

#[test]
fn the_verdict_reads_the_fleet_that_actually_ran() {
    let plan = in_process("fleet", None);
    let (report, _) = drive::run(&plan, binary()).unwrap();
    std::fs::remove_dir_all(&plan.root).unwrap();
    assert_eq!(report.requested.terminals, 3);
    assert_eq!(report.requested.viewers, 6);
    assert_eq!(report.realized, report.requested);
    let shape_checks: Vec<&str> = report
        .verdict
        .violations
        .iter()
        .map(|violation| violation.check)
        .filter(|check| check.ends_with("_realized"))
        .collect();
    assert_eq!(shape_checks, Vec::<&str>::new());
}

#[test]
fn a_teardown_step_that_never_finishes_fails_naming_its_terminal() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let bound = Duration::from_millis(20);
    let stalled = runtime
        .block_on(settle::bounded(
            ("actor", 7),
            bound,
            std::future::pending::<()>(),
        ))
        .unwrap_err();
    assert_eq!(stalled.kind(), "shutdown_stalled");
    assert!(
        matches!(
            stalled,
            Terminal50Error::ShutdownStalled {
                stage: "actor",
                index: 7,
                bound_ms: 20
            }
        ),
        "{stalled:?}"
    );
    let finished = runtime.block_on(settle::bounded(("actor", 7), bound, std::future::ready(5)));
    assert_eq!(finished.unwrap(), 5);
}
