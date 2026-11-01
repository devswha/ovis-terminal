//! Seeded fixtures: one known defect per gate rule plus a clean input per gate, so every
//! gate is shown to fail on what it guards and to pass on what it should accept.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::checklist::{coverage, dispositions};
use super::error::QualityError;
use super::ideal_audit::IdealRequirements;
use super::ledgers::Ledgers;
use super::matrix::{IdealMap, Sources};
use super::matrix_build::{Inputs, build};
use super::plan::parse;
use super::plan_audit::Requirements;
use super::report::Finding;
use super::scan::{SourceLimits, check_text};
use super::violations;

/// Names `source-check --fixture` accepts.
pub(super) const SOURCE_FIXTURES: &[&str] = &[
    "clean-source",
    "oversized-file",
    "fourth-parameter",
    "dropped-result",
    "allow-attribute",
    "numeric-cast",
    "tautological-assert",
    "checklist-missing-rule",
];

const UNLISTED_RULE: &str = "## Rule totals\n\n| rule id | rows | inherit-risk yes | highest severity | what ovis-terminal does instead |\n|---|---:|---:|---|---|\n| UNLISTED-RULE | 1 | 1 | high | A rule no disposition covers. |\n\n## Source violations\n\n| id | rule | severity | location | evidence | why | inherit-risk |\n|---|---|---|---|---|---|---|\n| V-9001 | UNLISTED-RULE | high | seed.rs:1 | `x` | Seeded. | yes |\n";

fn unknown(name: &str, known: &'static [&'static str]) -> QualityError {
    QualityError::UnknownFixture {
        name: name.to_owned(),
        known,
    }
}

fn source_text(name: &str) -> Option<String> {
    let text = match name {
        "clean-source" => {
            "fn add(left: u32, right: u32) -> u32 {\n    left.saturating_add(right)\n}\n".to_owned()
        }
        "oversized-file" => (1..=251)
            .map(|line| format!("const C{line}: u32 = {line};\n"))
            .collect::<Vec<_>>()
            .concat(),
        "fourth-parameter" => {
            "fn four(a: u8, b: u8, c: u8, d: u8) -> u8 {\n    a ^ b ^ c ^ d\n}\n".to_owned()
        }
        "dropped-result" => {
            "fn tidy() {\n    let _ = std::fs::remove_file(\"stale.lock\");\n}\n".to_owned()
        }
        "allow-attribute" => "#[allow(dead_code)]\nfn unused() {}\n".to_owned(),
        "numeric-cast" => "fn narrow(value: u64) -> u32 {\n    value as u32\n}\n".to_owned(),
        "tautological-assert" => {
            "#[test]\nfn status_moves() {\n    let since = 3;\n    assert!(since >= since);\n}\n"
                .to_owned()
        }
        _ => return None,
    };
    Some(text)
}

/// The seeded file's report path and its findings.
pub(super) fn source(
    name: &str,
    limits: SourceLimits,
) -> Result<(String, Vec<Finding>), QualityError> {
    let path = format!("seed/{name}.rs");
    if name == "checklist-missing-rule" {
        let (ledger, mut found) = violations::parse(UNLISTED_RULE);
        let table = dispositions().map_err(|finding| QualityError::Ledger {
            path: PathBuf::from(finding.location),
            missing: finding.detail,
        })?;
        found.extend(
            coverage(&table, &ledger)
                .into_iter()
                .filter(|finding| finding.location == "UNLISTED-RULE"),
        );
        return Ok(("seed/violations.md".to_owned(), found));
    }
    let text = source_text(name).ok_or_else(|| unknown(name, SOURCE_FIXTURES))?;
    Ok((path.clone(), check_text(&path, &text, limits)))
}

/// Names `evidence-audit --fixture` accepts.
pub(super) const EVIDENCE_FIXTURES: &[&str] = &[
    "clean-evidence",
    "missing-receipt-and-pass-banner",
    "prose-pass-banner",
    "fake-exit-zero",
    "forged-assertion",
    "skipped-as-pass",
];

/// The seeded evidence document's label and text.
pub(super) fn evidence(name: &str) -> Result<(String, String), QualityError> {
    let text = match name {
        "clean-evidence" => include_str!("fixtures/evidence/clean-evidence.json"),
        "missing-receipt-and-pass-banner" => {
            include_str!("fixtures/evidence/missing-receipt-and-pass-banner.json")
        }
        "prose-pass-banner" => include_str!("fixtures/evidence/prose-pass-banner.txt"),
        "fake-exit-zero" => include_str!("fixtures/evidence/fake-exit-zero.json"),
        "forged-assertion" => include_str!("fixtures/evidence/forged-assertion.json"),
        "skipped-as-pass" => include_str!("fixtures/evidence/skipped-as-pass.json"),
        _ => return Err(unknown(name, EVIDENCE_FIXTURES)),
    };
    Ok((format!("seed/{name}"), text.to_owned()))
}

/// Names `plan-audit --fixture` accepts.
pub(super) const PLAN_FIXTURES: &[&str] = &[
    "clean-plan",
    "missing-qa",
    "unresolved-reference",
    "uncovered-gap",
    "overlapping-writes",
];

fn ledgers() -> Result<Ledgers, QualityError> {
    let ideal_state = include_str!("fixtures/plan/ideal-state.md");
    let gap_list = include_str!("fixtures/plan/gap-list.md");
    Ledgers::parse(ideal_state, gap_list).map_err(|missing| QualityError::Ledger {
        path: PathBuf::from("probes/quality/fixtures/plan"),
        missing: missing.to_owned(),
    })
}

fn plan_text(name: &str) -> Option<&'static str> {
    Some(match name {
        "clean-plan" => include_str!("fixtures/plan/clean.md"),
        "missing-qa" => include_str!("fixtures/plan/missing-qa.md"),
        "unresolved-reference" => include_str!("fixtures/plan/unresolved-reference.md"),
        "uncovered-gap" => include_str!("fixtures/plan/uncovered-gap.md"),
        "overlapping-writes" => include_str!("fixtures/plan/overlapping-writes.md"),
        _ => return None,
    })
}

/// The seeded plan, its two-row ledgers and requirements that cover exactly them.
pub(super) fn plan(name: &str) -> Result<(String, Ledgers, Requirements), QualityError> {
    let text = plan_text(name).ok_or_else(|| unknown(name, PLAN_FIXTURES))?;
    let requirements = Requirements {
        todos: (1, 2),
        ideals: 2,
        gaps: 2,
        exclusions: 1,
        reject_unresolved: true,
        scope_root: PathBuf::from("/seed"),
        deny_write_roots: vec![PathBuf::from("/seed/ovis")],
    };
    Ok((text.to_owned(), ledgers()?, requirements))
}

/// Names `ideal-audit --fixture` accepts.
pub(super) const IDEAL_FIXTURES: &[&str] = &["clean-proof", "unproved-ideal", "forged-proof"];

fn passing_run(scenario: &str, cases: &[&str]) -> Value {
    let assertion = json!({"check": "seed/held", "expected": 1, "actual": 1, "passed": true});
    let cases: Vec<Value> = cases
        .iter()
        .map(|case| json!({"scenario": scenario, "case": case, "outcome": "passed", "failure": null, "assertions": [assertion], "processes": []}))
        .collect();
    json!({
        "format": "ovis-conformance-evidence/1",
        "host": {"os": "linux", "arch": "x86_64"},
        "binary": {"path": "target/debug/ovis-terminal", "sha256": "a".repeat(64)},
        "result": "passed",
        "error": null,
        "totals": {"cases": cases.len().to_string(), "passed": cases.len().to_string(), "failed": "0", "skipped": "0", "assertions": cases.len().to_string()},
        "cases": cases,
    })
}

type IdealSeed = (IdealMap, Ledgers, Vec<(String, Value)>, IdealRequirements);

/// The seeded map (built from the clean plan), its ledgers, evidence runs and requirements.
pub(super) fn ideal(name: &str) -> Result<IdealSeed, QualityError> {
    let mut runs = vec![
        (
            "seed/targets".to_owned(),
            passing_run("seed/targets", &["happy", "failure"]),
        ),
        (
            "seed/waits".to_owned(),
            passing_run("seed/waits", &["happy", "failure"]),
        ),
    ];
    match name {
        "clean-proof" => {}
        "unproved-ideal" => runs.truncate(1),
        "forged-proof" => {
            if let Some(actual) = runs
                .last_mut()
                .and_then(|(_, run)| run.pointer_mut("/cases/1/assertions/0/actual"))
            {
                *actual = json!(2);
            }
        }
        _ => return Err(unknown(name, IDEAL_FIXTURES)),
    }
    let ledgers = ledgers()?;
    let plan = parse(include_str!("fixtures/plan/clean.md"));
    let schema = r#"{"info": {"x-generation": "1"}, "methods": [{"name": "seed.ping"}]}"#;
    let sources = Sources {
        plan: "seed".to_owned(),
        plan_sha256: String::new(),
        ideal_state_sha256: String::new(),
        gap_list_sha256: String::new(),
        command: String::new(),
    };
    let inputs = Inputs {
        plan: &plan,
        ledgers: &ledgers,
        schema_path: "seed.json",
        schema_text: schema,
        sources,
    };
    let (_, map) = build(&inputs).map_err(|source| QualityError::Json {
        path: PathBuf::from("seed.json"),
        source,
    })?;
    let requirements = IdealRequirements {
        rows: 2,
        platforms: vec!["linux".to_owned()],
        windows_smoke: false,
        soak_seconds: 0,
    };
    Ok((map, ledgers, runs, requirements))
}
