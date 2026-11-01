//! Every quality gate fails on its seeded defects, passes clean inputs and never loosens
//! the binding limits.

pub mod support;

use ovis_terminal_probes::quality::Tool;
use serde_json::Value;

use self::support::{field, run};

const SEEDED_DEFECTS: &[(Tool, &str, &str, &str)] = &[
    (
        Tool::SourceCheck,
        "oversized-file",
        "quality_violation",
        "max-pure-loc",
    ),
    (
        Tool::SourceCheck,
        "fourth-parameter",
        "quality_violation",
        "max-parameters",
    ),
    (
        Tool::SourceCheck,
        "dropped-result",
        "quality_violation",
        "dropped-result",
    ),
    (
        Tool::SourceCheck,
        "allow-attribute",
        "quality_violation",
        "allow-attribute",
    ),
    (
        Tool::SourceCheck,
        "numeric-cast",
        "quality_violation",
        "numeric-cast",
    ),
    (
        Tool::SourceCheck,
        "tautological-assert",
        "quality_violation",
        "tautological-assert",
    ),
    (
        Tool::SourceCheck,
        "ok-discard",
        "quality_violation",
        "dropped-result",
    ),
    (
        Tool::SourceCheck,
        "checklist-missing-rule",
        "quality_violation",
        "checklist-disposition",
    ),
    (
        Tool::EvidenceAudit,
        "missing-receipt-and-pass-banner",
        "evidence_invalid",
        "binary-hash",
    ),
    (
        Tool::EvidenceAudit,
        "prose-pass-banner",
        "evidence_invalid",
        "manifest-schema",
    ),
    (
        Tool::EvidenceAudit,
        "fake-exit-zero",
        "evidence_invalid",
        "exit-forged",
    ),
    (
        Tool::EvidenceAudit,
        "forged-assertion",
        "evidence_invalid",
        "assertion-forged",
    ),
    (
        Tool::EvidenceAudit,
        "skipped-as-pass",
        "evidence_invalid",
        "pass-claim-unsupported",
    ),
    (
        Tool::EvidenceAudit,
        "forged-matcher",
        "evidence_invalid",
        "unknown-matcher",
    ),
    (
        Tool::EvidenceAudit,
        "present-on-null",
        "evidence_invalid",
        "assertion-forged",
    ),
    (
        Tool::EvidenceAudit,
        "forged-gate-report",
        "evidence_invalid",
        "gate-report-forged",
    ),
    (
        Tool::PlanAudit,
        "missing-qa",
        "plan_invalid",
        "missing-scenario",
    ),
    (
        Tool::PlanAudit,
        "unresolved-reference",
        "plan_invalid",
        "unresolved-ref",
    ),
    (
        Tool::PlanAudit,
        "uncovered-gap",
        "plan_invalid",
        "uncovered-gap",
    ),
    (
        Tool::PlanAudit,
        "overlapping-writes",
        "plan_invalid",
        "write-conflict",
    ),
    (
        Tool::IdealAudit,
        "unproved-ideal",
        "ideal_unproved",
        "unproved-ideal",
    ),
    (
        Tool::IdealAudit,
        "forged-proof",
        "ideal_unproved",
        "evidence-invalid",
    ),
];

#[test]
fn every_seeded_defect_fails_its_gate_with_exit_one_and_its_rule() {
    for (tool, fixture, kind, rule) in SEEDED_DEFECTS {
        let ran = run(*tool, &["--fixture", fixture]);
        assert_eq!(ran.exit, 1, "{fixture}: {}", ran.report);
        assert_eq!(field(&ran.report, "/kind"), kind, "{fixture}");
        assert_eq!(field(&ran.report, "/verdict"), "fail", "{fixture}");
        assert_eq!(
            field(&ran.report, "/findings/0/rule"),
            rule,
            "{fixture}: {}",
            ran.report
        );
    }
}

#[test]
fn every_clean_seed_passes_with_no_finding() {
    let clean = [
        (Tool::SourceCheck, "clean-source"),
        (Tool::EvidenceAudit, "clean-evidence"),
        (Tool::PlanAudit, "clean-plan"),
        (Tool::IdealAudit, "clean-proof"),
    ];
    for (tool, fixture) in clean {
        let ran = run(tool, &["--fixture", fixture]);
        assert_eq!(ran.exit, 0, "{fixture}: {}", ran.report);
        assert_eq!(
            field(&ran.report, "/findings"),
            &Value::Array(Vec::new()),
            "{fixture}"
        );
        assert_eq!(field(&ran.report, "/kind"), &Value::Null, "{fixture}");
    }
}

#[test]
fn limits_may_be_tightened_but_never_loosened() {
    for (flag, value) in [("--max-pure-loc", "251"), ("--max-params", "4")] {
        let ran = run(
            Tool::SourceCheck,
            &["--fixture", "clean-source", flag, value],
        );
        assert_eq!(ran.exit, 2, "{flag}: {}", ran.report);
        assert_eq!(field(&ran.report, "/kind"), "limit_above_contract");
    }
    let tightened = run(
        Tool::SourceCheck,
        &["--fixture", "clean-source", "--max-params", "1"],
    );
    assert_eq!(tightened.exit, 1, "{}", tightened.report);
    assert_eq!(
        field(&tightened.report, "/findings/0/rule"),
        "max-parameters"
    );
}

#[test]
fn an_unknown_fixture_or_flag_is_a_usage_error_naming_the_choices() {
    let ran = run(Tool::EvidenceAudit, &["--fixture", "nonexistent"]);
    assert_eq!(ran.exit, 2);
    assert_eq!(field(&ran.report, "/kind"), "unknown_fixture");
    assert!(
        field(&ran.report, "/detail")
            .as_str()
            .is_some_and(|text| text.contains("clean-evidence"))
    );
    let flag = run(
        Tool::PlanAudit,
        &["--fixture", "clean-plan", "--no-such-flag"],
    );
    assert_eq!(
        (flag.exit, field(&flag.report, "/kind").clone()),
        (2, Value::from("usage"))
    );
}
