//! Suite inventory and the runner's own exit code, against real and purpose-built trees.

use std::path::Path;

use serde_json::Value;

use super::support::{TestResult, Tree, conformance, field, scenario, strings};

const EXECUTABLE: &[&str] = &[
    "_runner/self-test.json",
    "agent/binding.json",
    "agent/bridge.json",
    "agent/e2e.json",
    "agent/launch.json",
    "agent/prompts.json",
    "agent/records.json",
    "agent/status.json",
    "agent/tree.json",
    "all/all.json",
    "cli/interactive-attach.json",
    "cli/manual.json",
    "client/package.json",
    "observability/health.json",
    "part-a/task-21.json",
    "part-a/task-29.json",
    "part-a/task-77.json",
    "part-a/task-78.json",
    "part-a/task-79.json",
    "part-a/task-86.json",
    "platform/matrix.json",
    "platform/process-info.json",
    "platform/unix-service.json",
    "protocol/deadline.json",
    "protocol/discovery.json",
    "protocol/fragmentation.json",
    "protocol/multiplex.json",
    "protocol/pagination.json",
    "protocol/strict-target.json",
    "quality/gates.json",
    "recovery/e2e.json",
    "remote/e2e.json",
    "remote/isolation.json",
    "security/rotation.json",
    "security/validation.json",
    "services/update.json",
];

/// Executable scenarios whose server is the assembled binary itself (plan todo 31).
const ASSEMBLED: [&str; 7] = [
    "graphics/passthrough.json",
    "part-a/task-22.json",
    "part-a/task-26.json",
    "part-a/task-27.json",
    "part-a/task-31.json",
    "recovery/restart.json",
    "services/worktrees.json",
];

/// Every executable scenario file, sorted as the inventory reports them.
fn executable() -> Vec<String> {
    let mut files: Vec<String> = EXECUTABLE
        .iter()
        .chain(&ASSEMBLED)
        .map(|file| (*file).to_owned())
        .collect();
    files.sort();
    files
}

/// Every executable file runs and passes; every other file is a listed declaration.
#[test]
fn every_scenario_file_is_executed_or_a_listed_declaration() -> TestResult {
    let ran = conformance("all/all", "all", &[])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    let summary: Value = serde_json::from_slice(&ran.output.stdout)?;
    assert_eq!(
        (
            field(&summary, "/evidence_written"),
            field(&summary, "/evidence_error")
        ),
        (&Value::Bool(true), &Value::Null),
        "the summary reports the evidence file it wrote"
    );
    let evidence = &ran.evidence;
    assert_eq!(strings(evidence, "/inventory/executed"), executable());
    let allowlist: Value = serde_json::from_slice(&std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios/declared.json"),
    )?)?;
    let listed = strings(&allowlist, "/entries");
    assert_ne!(listed, Vec::<String>::new());
    assert_eq!(strings(evidence, "/inventory/declared_only"), listed);
    for empty in [
        "/inventory/invalid",
        "/inventory/unlisted",
        "/inventory/stale",
    ] {
        assert_eq!(strings(evidence, empty), Vec::<String>::new(), "{empty}");
    }
    assert_eq!(
        *field(evidence, "/totals/declared_only"),
        Value::from(listed.len().to_string())
    );
    assert_eq!(field(evidence, "/totals/skipped"), "0");
    let ran_ids: std::collections::BTreeSet<&str> = field(evidence, "/cases")
        .as_array()
        .ok_or("cases")?
        .iter()
        .filter_map(|case| field(case, "/scenario").as_str())
        .collect();
    for file in executable().iter().filter(|file| *file != "all/all.json") {
        let id = file.trim_end_matches(".json");
        assert!(ran_ids.contains(id), "{id} did not run");
    }
    Ok(())
}

/// The final wave's `--require-no-skips` cannot pass while declarations remain.
#[test]
fn require_no_skips_fails_while_declarations_remain() -> TestResult {
    let ran = conformance("all/all", "all", &["--require-no-skips"])?;
    assert_eq!(ran.output.status.code(), Some(1), "{}", ran.explain());
    assert_eq!(field(&ran.evidence, "/result"), "failed");
    assert_eq!(field(&ran.evidence, "/error/kind"), "declared_only_present");
    Ok(())
}

/// A case whose assertion fails makes the whole process exit 1 with result failed.
#[test]
fn a_failing_case_fails_the_process() -> TestResult {
    let tree = Tree::new("failing", &[("zz/failing.json", scenario("zz/failing", 3))])?;
    let ran = conformance("zz/failing", "happy", &["--scenario-root", &tree.arg()])?;
    assert_eq!(ran.output.status.code(), Some(1), "{}", ran.explain());
    assert_eq!(field(&ran.evidence, "/result"), "failed");
    assert_eq!(field(&ran.evidence, "/totals/failed"), "1");
    assert_eq!(field(&ran.evidence, "/cases/0/outcome"), "failed");
    assert_eq!(
        field(&ran.evidence, "/cases/0/failure/kind"),
        "assertion_failed"
    );
    let passing = Tree::new("passing", &[("zz/passing.json", scenario("zz/passing", 0))])?;
    let ran = conformance("zz/passing", "happy", &["--scenario-root", &passing.arg()])?;
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain());
    Ok(())
}

/// A misspelled marker, a name outside the grammar, a non-JSON file, an unlisted
/// declaration and a stale allowlist entry are each reported, and the suite fails.
#[test]
fn invalid_unlisted_and_stale_files_fail_the_suite() -> TestResult {
    let suite = serde_json::json!({"format": "ovis-conformance/1", "scenario": "all/all",
        "title": "suite", "todo": 21, "platforms": ["linux", "macos"], "suite": "registered"});
    let allowlist = serde_json::json!({"format": "ovis-declared-scenarios/1", "entries": [
        {"file": "zz/gone.json", "owner_todo": "99", "reason": "removed"}]});
    let typo = scenario("zz/typo", 3).replace("ovis-conformance/1", "ovis-conformance/l");
    let tree = Tree::new(
        "inventory",
        &[
            ("all/all.json", suite.to_string()),
            ("declared.json", allowlist.to_string()),
            ("zz/ok.json", scenario("zz/ok", 0)),
            ("zz/typo.json", typo),
            ("zz/Zz_Bad.json", scenario("zz/Zz_Bad", 3)),
            ("zz/notes.txt", "not a scenario".to_owned()),
            (
                "zz/undeclared.json",
                serde_json::json!({"todo": 99, "steps": []}).to_string(),
            ),
        ],
    )?;
    let ran = conformance("all/all", "all", &["--scenario-root", &tree.arg()])?;
    assert_eq!(ran.output.status.code(), Some(1), "{}", ran.explain());
    let evidence = &ran.evidence;
    assert_eq!(field(evidence, "/error/kind"), "scenario_invalid");
    assert_eq!(
        strings(evidence, "/inventory/invalid"),
        ["zz/Zz_Bad.json", "zz/notes.txt", "zz/typo.json"]
    );
    assert_eq!(
        strings(evidence, "/inventory/unlisted"),
        ["zz/undeclared.json"]
    );
    assert_eq!(strings(evidence, "/inventory/stale"), ["zz/gone.json"]);
    assert_eq!(
        strings(evidence, "/inventory/executed"),
        ["all/all.json", "zz/ok.json"]
    );
    assert_eq!(field(evidence, "/cases/0/scenario"), "zz/ok");
    assert_eq!(field(evidence, "/cases/0/outcome"), "passed");
    Ok(())
}
