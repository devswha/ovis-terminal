//! Narrowing beneath references, nullable wrappers, intersections and maps, and
//! bounded reference traversal (gate review round 2).

use std::fs::File;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use ovis_terminal_protocol::schema::common::PageRequest;
use ovis_terminal_protocol::schema::{breaking_changes, openrpc_document};
use serde_json::{Value, json};

#[cfg(test)]
fn released() -> &'static Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(|| openrpc_document().expect("the catalog should generate"))
}

#[cfg(test)]
fn with(base: &Value, pointer: &str, value: Value) -> Value {
    let mut document = base.clone();
    *document
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("{pointer} exists")) = value;
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

#[cfg(test)]
fn cols(document: &Value, extra: &Value) -> Value {
    let mut cols = document
        .pointer("/components/schemas/Geometry/properties/cols")
        .cloned()
        .expect("cols");
    if let (Some(target), Some(source)) = (cols.as_object_mut(), extra.as_object()) {
        target.extend(source.clone());
    }
    with(
        document,
        "/components/schemas/Geometry/properties/cols",
        cols,
    )
}

#[test]
fn numeric_enum_losing_a_value_is_narrowed() {
    let old = cols(released(), &json!({"enum": [80, 120]}));
    let new = cols(released(), &json!({"enum": [80]}));
    let found = rule_tokens(&old, &new);
    assert!(
        found.contains(&(
            "terminal.run(geometry).cols".to_owned(),
            "input_values_narrowed"
        )),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .all(|(_, rule)| *rule == "input_values_narrowed"),
        "{found:?}"
    );
    let reverse = rule_tokens(&new, &old);
    assert!(
        reverse.iter().all(|(_, rule)| !rule.starts_with("input_")),
        "adding a numeric value widens inputs: {reverse:?}"
    );
    assert!(
        reverse
            .iter()
            .any(|(_, rule)| *rule == "closed_output_enum_grew"),
        "the same field is a closed output enum"
    );
}

#[test]
fn pattern_beneath_a_nullable_reference_is_narrowed() {
    let accepted: PageRequest =
        serde_json::from_value(json!({"after": "old-token", "limit": 1})).expect("released input");
    assert!(accepted.after.is_some());
    let mut token = released()
        .pointer("/components/schemas/PageToken")
        .cloned()
        .expect("PageToken");
    token
        .as_object_mut()
        .expect("object")
        .insert("pattern".to_owned(), json!("^p[0-9]+$"));
    let new = with(released(), "/components/schemas/PageToken", token);
    let found = rule_tokens(released(), &new);
    assert!(
        found.contains(&(
            "terminal.list(page).after".to_owned(),
            "input_pattern_changed"
        )),
        "{found:?}"
    );
}

#[test]
fn intersection_part_that_lowers_a_bound_is_narrowed() {
    let new = cols(released(), &json!({"allOf": [{"maximum": 200}]}));
    assert!(
        rule_tokens(released(), &new)
            .iter()
            .any(|(_, rule)| *rule == "input_bound_narrowed")
    );
}

#[test]
fn map_value_schemas_are_compared() {
    let labels = "/components/schemas/EventFilter/properties/labels";
    let base = with(
        released(),
        labels,
        json!({"type": ["object", "null"], "additionalProperties": {"type": "string"}}),
    );
    let narrowed = with(
        &base,
        labels,
        json!({"type": ["object", "null"], "additionalProperties": {"type": "string", "maxLength": 8}}),
    );
    let found = rule_tokens(&base, &narrowed);
    assert!(
        found
            .iter()
            .any(|(p, r)| p.ends_with("(filter).labels.*") && *r == "input_bound_narrowed"),
        "{found:?}"
    );
    let patterned = with(
        &base,
        labels,
        json!({"type": "object", "patternProperties": {"^x": {"type": "integer"}}}),
    );
    let found = rule_tokens(
        &patterned,
        &with(
            &patterned,
            &format!("{labels}/patternProperties/^x"),
            json!({"type": "integer", "maximum": 3}),
        ),
    );
    assert!(
        found.iter().any(|(_, r)| *r == "input_bound_narrowed"),
        "{found:?}"
    );
}

#[test]
fn nullable_array_items_are_compared() {
    let terminals = "/components/schemas/EventFilter/properties/terminals/items";
    let new = with(
        released(),
        terminals,
        json!({"type": "string", "maxLength": 3}),
    );
    let found = rule_tokens(released(), &new);
    assert!(
        found
            .iter()
            .any(|(p, r)| p == "events.subscribe(filter).terminals[]"
                && *r == "input_bound_narrowed"),
        "{found:?}"
    );
}

#[test]
fn self_referencing_schema_is_a_typed_cycle() {
    let cyclic = with(
        released(),
        "/components/schemas/Target",
        json!({"$ref": "#/components/schemas/Target"}),
    );
    let error = breaking_changes(&cyclic, released()).expect_err("a cycle cannot be walked");
    assert_eq!(error.kind(), "schema_reference_cycle");
}

#[test]
fn reference_chain_beyond_the_bound_is_a_typed_depth_error() {
    let mut chained = released().clone();
    let schemas = chained
        .pointer_mut("/components/schemas")
        .and_then(Value::as_object_mut)
        .expect("schemas");
    for step in 0..100 {
        schemas.insert(
            format!("Chain{step}"),
            json!({"$ref": format!("#/components/schemas/Chain{}", step + 1)}),
        );
    }
    schemas.insert("Chain100".to_owned(), json!({"type": "string"}));
    let chained = with(
        &chained,
        "/components/schemas/TerminalId",
        json!({"$ref": "#/components/schemas/Chain0"}),
    );
    let error = breaking_changes(&chained, released()).expect_err("depth is bounded");
    assert_eq!(error.kind(), "schema_depth_exceeded");
}

#[test]
fn cli_refuses_a_cyclic_baseline_and_an_oversized_file_with_typed_errors() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/cyclic-released.json");
    let run = |path: &PathBuf| {
        Command::new(env!("CARGO_BIN_EXE_openrpc"))
            .arg("compat")
            .arg(path)
            .output()
            .expect("runs")
    };
    let cyclic = run(&fixture);
    assert_eq!(cyclic.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&cyclic.stderr).expect("stderr is JSON");
    assert_eq!(report.get("kind"), Some(&json!("schema_reference_cycle")));
    let oversized =
        std::env::temp_dir().join(format!("ovis-oversized-{}.json", std::process::id()));
    File::create(&oversized)
        .and_then(|file| file.set_len(64 * 1024 * 1024 + 1))
        .expect("sparse file");
    let refused = run(&oversized);
    std::fs::remove_file(&oversized).expect("cleanup");
    assert_eq!(refused.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&refused.stderr).expect("stderr is JSON");
    assert_eq!(report.get("kind"), Some(&json!("schema_file_too_large")));
}
