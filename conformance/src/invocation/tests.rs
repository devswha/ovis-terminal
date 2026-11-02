//! The run verdict: which totals and errors fail the process.
use super::{failed, injected};
use crate::evidence::{Failure, Totals};
use crate::ids::Decimal;

fn totals(cases: u64, failed: u64, skipped: u64) -> Totals {
    Totals {
        cases: Decimal(cases),
        passed: Decimal(cases.saturating_sub(failed).saturating_sub(skipped)),
        failed: Decimal(failed),
        skipped: Decimal(skipped),
        ..Totals::default()
    }
}

#[test]
fn any_failed_case_error_empty_run_or_required_skip_fails_the_run() {
    assert!(!failed(None, &totals(3, 0, 0), true));
    assert!(
        failed(None, &totals(3, 1, 0), false),
        "a failed case fails the run"
    );
    assert!(
        failed(None, &totals(0, 0, 0), false),
        "a run with no case fails"
    );
    assert!(
        failed(None, &totals(3, 0, 1), true),
        "a skip fails under --require-no-skips"
    );
    assert!(!failed(None, &totals(3, 0, 1), false));
    let error = Failure {
        kind: "scenario_invalid",
        hint: "",
        detail: String::new(),
        step: None,
    };
    assert!(
        failed(Some(&error), &totals(3, 0, 0), false),
        "a top-level error fails the run"
    );
}

#[test]
fn an_inject_is_a_json_object_or_a_command_line_and_never_malformed_json() {
    assert_eq!(
        injected(r#"{"method":"agent.get"}"#.to_owned()),
        Some(serde_json::json!({"method": "agent.get"}))
    );
    assert_eq!(
        injected("ovis-terminal agent get --unknown-flag --json".to_owned()),
        Some(serde_json::json!(
            "ovis-terminal agent get --unknown-flag --json"
        ))
    );
    assert_eq!(injected(r#"{"method":"#.to_owned()), None, "truncated JSON");
    assert_eq!(injected("[1]".to_owned()), None, "a JSON array");
    assert_eq!(injected("7".to_owned()), None, "a JSON number");
}
