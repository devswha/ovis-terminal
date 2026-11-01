//! Process tests of the real runner binary driving the real `ovis-terminal` binary.
//!
//! The binary under test is `$OVIS_TERMINAL_BINARY`, or `ovis-terminal` next to the
//! runner in the same target directory (built by any workspace test run). A missing
//! binary fails the test; it is never skipped.
#![cfg(unix)]

use std::path::Path;

use serde_json::Value;

#[path = "runner_tests/attach_input.rs"]
mod attach_input;
#[path = "runner_tests/suite.rs"]
mod suite;
#[path = "runner_tests/support.rs"]
pub mod support;

use support::{TestResult, assertion, binary, conformance, field};

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
    for check in [
        "wire[oversized-header]/decode/whole",
        "wire[oversized-header]/decode/bytewise",
    ] {
        let decoded = assertion(evidence, check).ok_or(check)?;
        assert_eq!(field(decoded, "/passed"), true, "{check}");
        assert_eq!(
            field(decoded, "/actual/error"),
            "frame_too_large",
            "{check}"
        );
    }
    let sources: Vec<&Value> = field(evidence, "/cases/0/descriptors")
        .as_array()
        .ok_or("descriptors")?
        .iter()
        .map(|record| field(record, "/source"))
        .collect();
    assert!(
        sources.contains(&&Value::from("fixtures/protocol/strict-rejections.json")),
        "{sources:?}"
    );
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
