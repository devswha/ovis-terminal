//! Auditing ideal-state fidelity: an IDEAL row is proved only by audited, passing evidence
//! for every scenario mapped to it on every required platform.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::evidence::audit_value;
use super::ledgers::{Ledgers, required};
use super::matrix::IdealMap;
use super::report::{FailureKind, Finding, Verdict, count};

pub(super) const IDEAL_UNPROVED: FailureKind = FailureKind {
    kind: "ideal_unproved",
    hint: "run each mapped scenario's happy and failure case on every required platform and pass the evidence directory",
};

const SOAK_FORMAT: &str = "ovis-soak-receipt/1";
const WINDOWS_SCENARIO: &str = "platform/windows-service";
const CASES: &[&str] = &["happy", "failure"];

/// What the audit demands beyond the mapping.
#[derive(Debug, Clone)]
pub(super) struct IdealRequirements {
    pub(super) rows: u16,
    pub(super) platforms: Vec<String>,
    pub(super) windows_smoke: bool,
    pub(super) soak_seconds: u64,
    /// Root that `$equals_file` operands in the evidence are read under.
    pub(super) repo: std::path::PathBuf,
}

/// `(scenario, case, os)` of every passed case in audited evidence.
type Passed = BTreeSet<(String, String, String)>;

fn collect(
    runs: &[(String, Value)],
    (verdict, wanted): (&mut Verdict, &IdealRequirements),
) -> (Passed, u64) {
    let mut passed = Passed::new();
    let mut soak = 0_u64;
    for (label, run) in runs {
        if run.get("format").and_then(Value::as_str) == Some(SOAK_FORMAT) {
            let seconds = run
                .get("continuous_seconds")
                .and_then(Value::as_str)
                .and_then(|text| text.parse().ok());
            soak = soak.max(seconds.unwrap_or(0));
            continue;
        }
        let problems = audit_value(label, run, &wanted.repo);
        if !problems.is_empty() {
            for problem in problems {
                verdict.findings.push(Finding::new(
                    "evidence-invalid",
                    problem.location,
                    format!("{}: {}", problem.rule, problem.detail),
                ));
            }
            continue;
        }
        let os = run
            .pointer("/host/os")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let cases = run
            .get("cases")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for case in cases
            .iter()
            .filter(|case| case.get("outcome").and_then(Value::as_str) == Some("passed"))
        {
            let field = |key: &str| {
                case.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            passed.insert((field("scenario"), field("case"), os.to_owned()));
        }
    }
    (passed, soak)
}

fn unproved(scenarios: &[String], passed: &Passed, platforms: &[String]) -> Vec<String> {
    let mut missing = Vec::new();
    for scenario in scenarios {
        for platform in platforms {
            for case in CASES {
                if !passed.contains(&(scenario.clone(), (*case).to_owned(), platform.clone())) {
                    missing.push(format!("{scenario} {case} on {platform}"));
                }
            }
        }
    }
    missing
}

/// The fidelity verdict, with `proved`, `missing` and `shortfalls` as F4 facts.
pub(super) fn audit(
    map: &IdealMap,
    ledgers: &Ledgers,
    evidence: (&[(String, Value)], &IdealRequirements),
) -> Verdict {
    let (runs, wanted) = evidence;
    let mut verdict = Verdict::new("ideal-audit", IDEAL_UNPROVED);
    let (passed, soak) = collect(runs, (&mut verdict, wanted));
    let mut missing = Vec::new();
    let mut proved = 0_usize;
    for id in required("IDEAL", wanted.rows) {
        let ledger = ledgers.ideals.iter().find(|ideal| ideal.id == id);
        let Some(entry) = map
            .ideals
            .get(&id)
            .filter(|entry| !entry.scenarios.is_empty())
        else {
            verdict.findings.push(Finding::new(
                "unmapped-ideal",
                id.clone(),
                "no scenario maps this row",
            ));
            missing.push(id);
            continue;
        };
        if ledger.is_none_or(|ideal| ideal.statement != entry.predicate) {
            verdict.findings.push(Finding::new(
                "predicate-drift",
                id.clone(),
                "the mapped predicate is not the ledger statement",
            ));
        }
        let gaps = unproved(&entry.scenarios, &passed, &wanted.platforms);
        if gaps.is_empty() {
            proved = proved.saturating_add(1);
        } else {
            verdict
                .findings
                .push(Finding::new("unproved-ideal", id.clone(), gaps.join("; ")));
            missing.push(id);
        }
    }
    let mut shortfalls = Vec::new();
    let mut gaps_closed = 0_usize;
    for gap in &ledgers.gaps {
        match map.gaps.get(&gap.id) {
            Some(entry) if !entry.todos.is_empty() && !entry.scenarios.is_empty() => {
                gaps_closed = gaps_closed.saturating_add(1);
            }
            Some(_) | None => shortfalls.push(Finding::new(
                "unmapped-gap",
                gap.id.clone(),
                "no owning todo and scenario",
            )),
        }
    }
    if wanted.windows_smoke
        && !unproved(
            &[WINDOWS_SCENARIO.to_owned()],
            &passed,
            &["windows".to_owned()],
        )
        .is_empty()
    {
        shortfalls.push(Finding::new(
            "windows-smoke",
            WINDOWS_SCENARIO,
            "no passing native Windows evidence",
        ));
    }
    if soak < wanted.soak_seconds {
        shortfalls.push(Finding::new(
            "soak-shortfall",
            SOAK_FORMAT,
            format!(
                "longest continuous soak {soak} s < {} s",
                wanted.soak_seconds
            ),
        ));
    }
    verdict.fact("proved", count(proved));
    verdict.fact("missing", json!(missing));
    verdict.fact("gaps_closed", count(gaps_closed));
    verdict.fact(
        "shortfalls",
        json!(
            shortfalls
                .iter()
                .map(|finding| format!("{}: {}", finding.location, finding.detail))
                .collect::<Vec<_>>()
        ),
    );
    verdict.findings.extend(shortfalls);
    verdict
}
