//! Constraints that compositions add: keywords beside a union and bounds inside
//! nested intersections apply to every value (gate review round 3).

use std::sync::OnceLock;

use ovis_terminal_protocol::schema::common::PageRequest;
use ovis_terminal_protocol::schema::{breaking_changes, openrpc_document};
use serde_json::{Value, json};

#[cfg(test)]
fn released() -> &'static Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(|| openrpc_document().expect("the catalog should generate"))
}

/// `base` with the members of `extra` merged into the schema at `pointer`.
#[cfg(test)]
fn extended(base: &Value, pointer: &str, extra: &Value) -> Value {
    let mut document = base.clone();
    let schema = document
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("{pointer} is an object schema"));
    schema.extend(extra.as_object().cloned().expect("extra is an object"));
    document
}

#[cfg(test)]
fn rule_tokens(old: &Value, new: &Value) -> Vec<(String, &'static str)> {
    let breaks = breaking_changes(old, new).expect("well formed");
    breaks
        .into_iter()
        .map(|b| (b.path, b.rule.token()))
        .collect()
}

const AFTER: &str = "/components/schemas/PageRequest/properties/after";
const COLS: &str = "/components/schemas/Geometry/properties/cols";

#[test]
fn pattern_beside_a_nullable_union_is_narrowed() {
    let accepted: PageRequest =
        serde_json::from_value(json!({"after": "old-token", "limit": 1})).expect("released input");
    assert!(
        accepted.after.is_some(),
        "the released input accepts old-token"
    );
    assert!(
        released().pointer(&format!("{AFTER}/anyOf")).is_some(),
        "after is a nullable union"
    );
    let new = extended(released(), AFTER, &json!({"pattern": "^p[0-9]+$"}));
    let found = rule_tokens(released(), &new);
    assert!(
        found.contains(&(
            "terminal.list(page).after".to_owned(),
            "input_pattern_changed"
        )),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .all(|(_, rule)| *rule == "input_pattern_changed"),
        "{found:?}"
    );
    assert_eq!(
        rule_tokens(&new, released()),
        [],
        "dropping the pattern widens inputs"
    );
}

#[test]
fn bound_inside_a_nested_intersection_is_narrowed() {
    let new = extended(
        released(),
        COLS,
        &json!({"allOf": [{"allOf": [{"maximum": 80}]}]}),
    );
    let found = rule_tokens(released(), &new);
    assert!(
        found.contains(&(
            "terminal.run(geometry).cols".to_owned(),
            "input_bound_narrowed"
        )),
        "{found:?}"
    );
    let looser = extended(
        released(),
        COLS,
        &json!({"allOf": [{"allOf": [{"maximum": 70000}]}]}),
    );
    assert_eq!(
        rule_tokens(released(), &looser),
        [],
        "a looser nested bound narrows nothing"
    );
}

#[test]
fn combinatorial_composition_is_a_typed_bound_error() {
    let pair = json!({"anyOf": [{"type": "integer"}, {"type": "number"}]});
    let parts: Vec<Value> = std::iter::repeat_n(pair, 9).collect();
    let exploding = extended(released(), COLS, &json!({ "allOf": parts }));
    let error = breaking_changes(released(), &exploding).expect_err("2^9 combinations exceed 256");
    assert_eq!(error.kind(), "schema_alternatives_exceeded");
    let parts: Vec<Value> = std::iter::repeat_n(
        json!({"anyOf": [{"type": "integer"}, {"type": "number"}]}),
        8,
    )
    .collect();
    let bounded = extended(released(), COLS, &json!({ "allOf": parts }));
    assert_eq!(
        rule_tokens(released(), &bounded),
        [],
        "256 integer-or-number combinations accept every integer"
    );
}
