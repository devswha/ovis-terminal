//! Auditing conformance evidence: a pass is accepted only when the document proves it.

use std::path::Path;

use serde_json::Value;

use super::gate_report;
use super::matchers::{Judgement, holds};
use super::report::{FailureKind, Finding};

/// Conformance runs and quality-gate reports are the two evidence entries accepted.
pub(super) const EVIDENCE_INVALID: FailureKind = FailureKind {
    kind: "evidence_invalid",
    hint: "rerun the conformance runner; a pass needs parsed exits, recomputable assertions and a binary hash",
};

const FORMAT: &str = "ovis-conformance-evidence/1";
const OUTCOMES: &[&str] = &["passed", "failed", "skipped"];

struct Audit<'run> {
    label: String,
    run: &'run Value,
    repo: &'run Path,
    found: Vec<Finding>,
}

impl Audit<'_> {
    fn flag(&mut self, rule: &'static str, detail: impl Into<String>) {
        self.found
            .push(Finding::new(rule, self.label.clone(), detail));
    }

    fn text(&self, pointer: &str) -> Option<&str> {
        self.run.pointer(pointer).and_then(Value::as_str)
    }

    fn decimal(&mut self, pointer: &str) -> Option<usize> {
        let value = self.text(pointer).and_then(|text| {
            let canonical = text == "0" || !text.starts_with('0');
            canonical.then(|| text.parse::<usize>().ok())?
        });
        if value.is_none() {
            self.flag(
                "manifest-schema",
                format!("{pointer} is not a canonical decimal string"),
            );
        }
        value
    }

    fn header(&mut self) {
        if self.text("/format") != Some(FORMAT) {
            self.flag("manifest-schema", format!("/format is not {FORMAT}"));
        }
        let hash = self.text("/binary/sha256").unwrap_or_default();
        let hex = hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        let claims_pass = self.text("/result") == Some("passed");
        if claims_pass && (!hex || self.text("/binary/path").is_none_or(str::is_empty)) {
            self.flag(
                "binary-hash",
                "the run claims a pass but names no binary path with a sha256 digest",
            );
        }
        if self.text("/host/os").is_none() || self.text("/host/arch").is_none() {
            self.flag("host", "the run does not record the host os and arch");
        }
        if !self
            .text("/result")
            .is_some_and(|result| OUTCOMES.contains(&result))
        {
            self.flag(
                "manifest-schema",
                "/result is not passed, failed or skipped",
            );
        }
    }
}

fn outcome_count(cases: &[Value], outcome: &str) -> usize {
    cases
        .iter()
        .filter(|case| case.get("outcome").and_then(Value::as_str) == Some(outcome))
        .count()
}

fn assertions(case: &Value) -> &[Value] {
    case.get("assertions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Exit codes the case's binary processes actually reported.
fn binary_exits(case: &Value) -> Vec<Value> {
    let processes = case
        .get("processes")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    processes
        .iter()
        .filter(|process| process.get("role").and_then(Value::as_str) == Some("binary"))
        .filter_map(|process| process.get("exit_code").cloned())
        .collect()
}

fn audit_case(audit: &mut Audit<'_>, case: &Value) {
    let name = format!(
        "{}/{}",
        case.get("scenario").and_then(Value::as_str).unwrap_or("?"),
        case.get("case").and_then(Value::as_str).unwrap_or("?")
    );
    let passed = case.get("outcome").and_then(Value::as_str) == Some("passed");
    let checked = assertions(case);
    if passed && checked.is_empty() {
        audit.flag(
            "receipt-missing",
            format!("{name} passed without a single assertion"),
        );
    }
    if passed && !case.get("failure").is_none_or(Value::is_null) {
        audit.flag(
            "pass-claim-unsupported",
            format!("{name} passed while recording a failure"),
        );
    }
    let exits = binary_exits(case);
    for assertion in checked {
        let (Some(check), Some(expected), Some(actual), Some(held)) = (
            assertion.get("check").and_then(Value::as_str),
            assertion.get("expected"),
            assertion.get("actual"),
            assertion.get("passed").and_then(Value::as_bool),
        ) else {
            audit.flag(
                "manifest-schema",
                format!("{name} has an assertion without check, expected, actual and passed"),
            );
            continue;
        };
        match holds(check, (expected, actual), audit.repo) {
            Judgement::Holds(recomputed) if recomputed != held => audit.flag(
                "assertion-forged",
                format!("{name} {check}: passed={held} but expected {expected} vs actual {actual}"),
            ),
            Judgement::Holds(_) => {}
            Judgement::UnknownMatcher(operator) => audit.flag(
                "unknown-matcher",
                format!("{name} {check}: {operator} is not a runner matcher, so the assertion proves nothing"),
            ),
            Judgement::Unjudgeable(why) => audit.flag(
                "unjudgeable-assertion",
                format!("{name} {check}: {why}"),
            ),
        }
        if passed && !held {
            audit.flag(
                "pass-claim-unsupported",
                format!("{name} passed with failed assertion {check}"),
            );
        }
        if check == "binary/exit" && !exits.contains(actual) {
            audit.flag(
                "exit-forged",
                format!("{name} asserts exit {actual} that no binary process reported"),
            );
        }
    }
}

fn audit_totals(audit: &mut Audit<'_>, cases: &[Value]) {
    let recorded = (
        audit.decimal("/totals/cases"),
        audit.decimal("/totals/passed"),
        audit.decimal("/totals/failed"),
        audit.decimal("/totals/skipped"),
        audit.decimal("/totals/assertions"),
    );
    let actual = (
        Some(cases.len()),
        Some(outcome_count(cases, "passed")),
        Some(outcome_count(cases, "failed")),
        Some(outcome_count(cases, "skipped")),
        Some(cases.iter().map(|case| assertions(case).len()).sum()),
    );
    if recorded != actual {
        audit.flag(
            "totals-forged",
            format!("totals {recorded:?} differ from the cases {actual:?}"),
        );
    }
    let claims_pass = audit.text("/result") == Some("passed");
    let clean = actual.0 > Some(0) && actual.2 == Some(0) && actual.3 == Some(0);
    let error_free = audit.run.get("error").is_none_or(Value::is_null);
    if claims_pass && !(clean && error_free) {
        audit.flag(
            "pass-claim-unsupported",
            "result passed with no case, a failed or skipped case, or a run error",
        );
    }
}

fn audit_run(label: String, run: &Value, repo: &Path) -> Vec<Finding> {
    if run.get("gate").is_some() {
        return gate_report::audit(&label, run);
    }
    let mut audit = Audit {
        label,
        run,
        repo,
        found: Vec::new(),
    };
    audit.header();
    let Some(cases) = run.get("cases").and_then(Value::as_array) else {
        audit.flag("manifest-schema", "/cases is not an array");
        return audit.found;
    };
    audit_totals(&mut audit, cases);
    for case in cases {
        audit_case(&mut audit, case);
    }
    audit.found
}

/// The runs one evidence text holds: one object or an `--append` array of them.
pub(super) fn runs(text: &str) -> Option<Vec<Value>> {
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Array(runs) => Some(runs),
        run @ Value::Object(_) => Some(vec![run]),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

/// Every finding about one evidence document, reported under `label`; `$equals_file`
/// operands are read under `repo`.
pub(super) fn audit_text(label: &str, text: &str, repo: &Path) -> Vec<Finding> {
    let Some(runs) = runs(text) else {
        let detail = "not a JSON evidence document; a printed PASS banner is not evidence";
        return vec![Finding::new("manifest-schema", label, detail)];
    };
    if runs.is_empty() {
        return vec![Finding::new(
            "manifest-schema",
            label,
            "the document holds no run",
        )];
    }
    runs.iter()
        .enumerate()
        .flat_map(|(index, run)| audit_run(format!("{label}#{index}"), run, repo))
        .collect()
}

/// Every finding about one parsed run, for callers that already split a document.
pub(super) fn audit_value(label: &str, run: &Value, repo: &Path) -> Vec<Finding> {
    audit_run(label.to_owned(), run, repo)
}
