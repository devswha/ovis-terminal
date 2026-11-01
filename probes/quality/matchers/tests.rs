use std::path::Path;

use serde_json::{Value, json};

use super::{Judgement, holds};

fn judge(expected: &Value, actual: &Value) -> Judgement {
    holds(
        "c",
        (expected, actual),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

#[test]
fn operators_are_recomputed_from_the_recorded_operand() {
    assert_eq!(
        judge(&json!({"$at_least": "10"}), &json!("12")),
        Judgement::Holds(true)
    );
    assert_eq!(
        judge(&json!({"$at_least": "10"}), &json!("9")),
        Judgement::Holds(false)
    );
    assert_eq!(
        judge(&json!({"$at_least": "10"}), &json!(12)),
        Judgement::Holds(false)
    );
    assert_eq!(
        judge(&json!({"$contains": [1]}), &json!([2, 1])),
        Judgement::Holds(true)
    );
    assert_eq!(
        judge(&json!({"$excludes": [1]}), &json!([2, 1])),
        Judgement::Holds(false)
    );
    assert_eq!(
        judge(&json!({"a": 1}), &json!({"a": 2})),
        Judgement::Holds(false)
    );
}

#[test]
fn presence_is_judged_from_the_recorded_null() {
    assert_eq!(
        judge(&json!({"$present": true}), &json!(null)),
        Judgement::Holds(false)
    );
    assert_eq!(
        judge(&json!({"$present": true}), &json!("hint")),
        Judgement::Holds(true)
    );
    assert_eq!(
        judge(&json!({"$present": false}), &json!(null)),
        Judgement::Holds(true)
    );
    assert_eq!(
        judge(&json!({"$capture": "id"}), &json!(null)),
        Judgement::Holds(false)
    );
}

#[test]
fn an_unknown_or_unreadable_operator_is_never_accepted() {
    assert_eq!(
        judge(&json!({"$bogus": "never"}), &json!("unrelated")),
        Judgement::UnknownMatcher("$bogus".to_owned())
    );
    assert!(matches!(
        judge(&json!({"$equals_file": "no/such/file.json"}), &json!({})),
        Judgement::Unjudgeable(_)
    ));
    assert_eq!(
        judge(&json!({"$equals_file": "Cargo.toml"}), &json!({})),
        Judgement::Unjudgeable(
            "Cargo.toml is not a readable JSON file under the repository".to_owned()
        )
    );
    let manifest = serde_json::from_str::<Value>(include_str!("../dispositions.json")).unwrap();
    assert_eq!(
        judge(
            &json!({"$equals_file": "quality/dispositions.json"}),
            &manifest
        ),
        Judgement::Holds(true)
    );
}

#[test]
fn an_expected_failure_compares_only_its_kind() {
    let actual = json!({"kind": "timeout", "detail": "300 ms"});
    let check =
        |expected: Value| holds("expect_failure/kind", (&expected, &actual), Path::new("."));
    assert_eq!(check(json!({"kind": "timeout"})), Judgement::Holds(true));
    assert_eq!(check(json!({"kind": "closed"})), Judgement::Holds(false));
}
