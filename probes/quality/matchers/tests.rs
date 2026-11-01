use serde_json::json;

use super::holds;

#[test]
fn operators_are_recomputed_from_the_recorded_operand() {
    assert_eq!(
        holds("c", &json!({"$at_least": "10"}), &json!("12")),
        Some(true)
    );
    assert_eq!(
        holds("c", &json!({"$at_least": "10"}), &json!("9")),
        Some(false)
    );
    assert_eq!(
        holds("c", &json!({"$at_least": "10"}), &json!(12)),
        Some(false)
    );
    assert_eq!(
        holds("c", &json!({"$contains": [1]}), &json!([2, 1])),
        Some(true)
    );
    assert_eq!(
        holds("c", &json!({"$excludes": [1]}), &json!([2, 1])),
        Some(false)
    );
    assert_eq!(holds("c", &json!({"a": 1}), &json!({"a": 2})), Some(false));
}

#[test]
fn operators_without_a_comparable_operand_are_not_judged() {
    assert_eq!(holds("c", &json!({"$present": true}), &json!(null)), None);
    assert_eq!(
        holds("c", &json!({"$equals_file": "x.json"}), &json!({})),
        None
    );
}

#[test]
fn an_expected_failure_compares_only_its_kind() {
    let actual = json!({"kind": "timeout", "detail": "300 ms"});
    assert_eq!(
        holds("expect_failure/kind", &json!({"kind": "timeout"}), &actual),
        Some(true)
    );
    assert_eq!(
        holds("expect_failure/kind", &json!({"kind": "closed"}), &actual),
        Some(false)
    );
}
