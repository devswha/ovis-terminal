//! Process tests of the real runner binary driving the real `ovis-terminal` binary.
//!
//! The binary under test is `$OVIS_TERMINAL_BINARY`, or `ovis-terminal` next to the
//! runner in the same target directory (built by any workspace test run). A missing
//! binary fails the test; it is never skipped.
#![cfg(unix)]

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn binary() -> TestResult<PathBuf> {
    let runner = Path::new(env!("CARGO_BIN_EXE_conformance"));
    let path = std::env::var_os("OVIS_TERMINAL_BINARY")
        .map_or_else(|| runner.with_file_name("ovis-terminal"), PathBuf::from);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "{} is missing: cargo build -p ovis-terminal-cli",
            path.display()
        )
        .into())
    }
}

struct Ran {
    output: Output,
    evidence: Value,
}

impl Ran {
    /// The summary line plus every failed case's typed failure and failed assertions.
    fn explain(&self) -> String {
        let mut lines = vec![String::from_utf8_lossy(&self.output.stdout).into_owned()];
        let cases = self.evidence.pointer("/cases").and_then(Value::as_array);
        for case in cases.into_iter().flatten() {
            if field(case, "/outcome") == "passed" {
                continue;
            }
            let (scenario, name) = (field(case, "/scenario"), field(case, "/case"));
            lines.push(format!("{scenario} {name}: {}", field(case, "/failure")));
            let assertions = field(case, "/assertions").as_array().into_iter().flatten();
            lines.extend(
                assertions
                    .filter(|assertion| field(assertion, "/passed") != true)
                    .map(|failed| format!("  {failed}")),
            );
        }
        lines.join("\n")
    }
}

fn field<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

fn conformance(scenario: &str, case: &str, extra: &[&str]) -> TestResult<Ran> {
    let dir = std::env::temp_dir().join(format!(
        "ovc-test-{}-{}-{case}",
        std::process::id(),
        scenario.replace('/', "-")
    ));
    std::fs::create_dir_all(&dir)?;
    let evidence = dir.join(format!("{case}.json"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_conformance"))
        .arg("--binary")
        .arg(binary()?)
        .args([
            "--scenario",
            scenario,
            "--isolated",
            "--case",
            case,
            "--evidence",
        ])
        .arg(&evidence)
        .args(extra)
        .output()?;
    let evidence = std::fs::read(&evidence).map_or(Value::Null, |bytes| {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    });
    std::fs::remove_dir_all(&dir)?;
    Ok(Ran { output, evidence })
}

fn assertion<'a>(evidence: &'a Value, check: &str) -> Option<&'a Value> {
    evidence
        .pointer("/cases")?
        .as_array()?
        .iter()
        .filter_map(|case| case.get("assertions")?.as_array())
        .flatten()
        .find(|assertion| assertion.get("check").and_then(Value::as_str) == Some(check))
}

/// One bridge connection carries interleaved requests whose replies match by id, the
/// held request answers after later ones, and every received byte is decoded.
#[test]
fn happy() -> TestResult {
    let ran = conformance("part-a/task-21", "happy", &[])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    let evidence = &ran.evidence;
    assert_eq!(field(evidence, "/result"), "passed");
    assert_eq!(field(evidence, "/totals/failed"), "0");
    let case = field(evidence, "/cases/0");
    assert_eq!(
        field(case, "/fixture/accepted"),
        "1",
        "exactly one connection was accepted"
    );
    let connections = field(case, "/connections")
        .as_array()
        .ok_or("connections")?;
    assert_eq!(connections.len(), 1);
    let only = connections.first().ok_or("one connection")?;
    assert_eq!(field(only, "/via"), "bridge");
    assert_eq!(
        field(only, "/bytes_received"),
        field(only, "/bytes_decoded")
    );
    let bridge = field(case, "/processes")
        .as_array()
        .ok_or("processes")?
        .iter()
        .find(|p| field(p, "/role") == "bridge")
        .ok_or("bridge")?;
    assert_eq!(field(bridge, "/exit_code"), 0);
    let held = assertion(evidence, "reply[held]/error/data/kind").ok_or("held reply checked")?;
    assert_eq!(field(held, "/actual"), "cancelled");
    assert!(
        assertion(evidence, "reply[set-alpha]/result/value")
            .is_some_and(|a| field(a, "/passed") == true)
    );
    Ok(())
}

/// A misspelled target and an oversized frame header get typed rejections, nothing is
/// written, and the server closes that connection with `frame_too_large`.
#[test]
fn failure() -> TestResult {
    let ran = conformance("part-a/task-21", "failure", &[])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    let evidence = &ran.evidence;
    let misspelled = assertion(
        evidence,
        "rejection[misspelled-target-field]/error/data/field_path",
    )
    .ok_or("field path")?;
    assert_eq!(field(misspelled, "/actual"), "params.target.terminal_idd");
    let effects = assertion(evidence, "effects/files").ok_or("effects checked")?;
    assert_eq!(*field(effects, "/actual"), serde_json::json!([]));
    let closing = assertion(evidence, "notification[$/closing]/params/reason").ok_or("closing")?;
    assert_eq!(field(closing, "/actual"), "frame_too_large");
    assert_eq!(
        field(evidence, "/cases/0/fixture/failures/frame_too_large"),
        "1"
    );
    Ok(())
}

#[test]
fn a_missing_scenario_fails_and_a_declaration_is_not_executable() -> TestResult {
    let missing = conformance("protocol/does-not-exist", "all", &[])?;
    assert_eq!(missing.output.status.code(), Some(1));
    assert_eq!(field(&missing.evidence, "/error/kind"), "scenario_missing");
    assert_eq!(field(&missing.evidence, "/result"), "failed");
    let declared = conformance("part-a/task-5", "happy", &[])?;
    assert_eq!(declared.output.status.code(), Some(1));
    assert_eq!(
        field(&declared.evidence, "/error/kind"),
        "scenario_not_executable"
    );
    Ok(())
}

#[test]
fn a_run_without_isolation_is_a_usage_error_before_anything_starts() -> TestResult {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_conformance"))
        .arg("--binary")
        .arg(binary()?)
        .args([
            "--scenario",
            "protocol/multiplex",
            "--evidence",
            "/nonexistent/never-written.json",
        ])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let summary: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(field(&summary, "/error/kind"), "isolation_required");
    assert!(!Path::new("/nonexistent/never-written.json").exists());
    Ok(())
}

/// The runner itself detects a wrong expectation, a timeout and a wrong exit status.
#[test]
fn runner_self_test_detects_its_own_failure_modes() -> TestResult {
    let ran = conformance("_runner/self-test", "all", &[])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    for kind in ["assertion_failed", "timeout"] {
        let detected = ran
            .evidence
            .pointer("/cases")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|case| case.get("assertions")?.as_array())
            .flatten()
            .any(|a| {
                field(a, "/check") == "expect_failure/kind" && field(a, "/actual/kind") == kind
            });
        assert!(detected, "self-test did not observe {kind}");
    }
    Ok(())
}

/// Every registered scenario runs, with no failure and no skip.
#[test]
fn every_registered_scenario_passes_with_no_skips() -> TestResult {
    let ran = conformance("all/all", "all", &["--require-no-skips"])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    let evidence = &ran.evidence;
    assert_eq!(field(evidence, "/totals/skipped"), "0");
    let scenarios: std::collections::BTreeSet<&str> = field(evidence, "/cases")
        .as_array()
        .ok_or("cases")?
        .iter()
        .filter_map(|case| field(case, "/scenario").as_str())
        .collect();
    for required in [
        "protocol/multiplex",
        "protocol/strict-target",
        "protocol/fragmentation",
        "protocol/discovery",
        "protocol/deadline",
        "protocol/pagination",
        "_runner/self-test",
    ] {
        assert!(scenarios.contains(required), "{required} did not run");
    }
    Ok(())
}
