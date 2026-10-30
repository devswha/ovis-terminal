//! Released-baseline rules applied to edits of the real generated document.

use std::sync::OnceLock;

use ovis_terminal_protocol::schema::methods::all_methods;
use ovis_terminal_protocol::schema::{Break, breaking_changes, openrpc_document};
use proptest::prelude::*;
use serde_json::{Value, json};

#[cfg(test)]
fn released() -> &'static Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(|| openrpc_document().expect("the catalog should generate"))
}

#[cfg(test)]
fn edited(edit: impl FnOnce(&mut Value)) -> Vec<Break> {
    let mut current = released().clone();
    edit(&mut current);
    breaking_changes(released(), &current).expect("both documents are well formed")
}

#[cfg(test)]
fn at<'a>(doc: &'a mut Value, pointer: &str) -> &'a mut Value {
    doc.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("{pointer} exists"))
}

#[cfg(test)]
fn definition<'a>(doc: &'a mut Value, name: &str) -> &'a mut Value {
    at(doc, &format!("/components/schemas/{name}"))
}

#[cfg(test)]
fn list<'a>(doc: &'a mut Value, pointer: &str) -> &'a mut Vec<Value> {
    at(doc, pointer)
        .as_array_mut()
        .unwrap_or_else(|| panic!("{pointer} is an array"))
}

#[cfg(test)]
fn insert(doc: &mut Value, pointer: &str, (key, value): (&str, Value)) {
    let object = at(doc, pointer)
        .as_object_mut()
        .unwrap_or_else(|| panic!("{pointer} is an object"));
    object.insert(key.to_owned(), value);
}

#[cfg(test)]
fn is_const(variant: &Value, value: &str) -> bool {
    variant.get("const").and_then(Value::as_str) == Some(value)
}

#[cfg(test)]
fn rules(breaks: &[Break]) -> Vec<&'static str> {
    breaks.iter().map(|b| b.rule.token()).collect()
}

#[cfg(test)]
fn method_params<'a>(doc: &'a mut Value, name: &str) -> &'a mut Vec<Value> {
    let methods = doc
        .get_mut("methods")
        .and_then(Value::as_array_mut)
        .expect("methods");
    let method = methods
        .iter_mut()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(name));
    method
        .and_then(|m| m.get_mut("params"))
        .and_then(Value::as_array_mut)
        .expect("params")
}

#[test]
fn unchanged_document_has_no_breaks() {
    assert_eq!(edited(|_| ()), Vec::<Break>::new());
}

#[test]
fn additive_changes_are_compatible() {
    let breaks = edited(|doc| {
        method_params(doc, "terminal.list")
            .push(json!({"name": "sort", "required": false, "schema": {"type": "string"}}));
        insert(
            doc,
            "/components/schemas/TerminalRecord/properties",
            ("cpu_ms", json!({"type": "string"})),
        );
        list(doc, "/components/schemas/TerminalState/oneOf")
            .push(json!({"type": "string", "const": "hibernating"}));
        insert(
            doc,
            "/components/schemas/Expect/properties",
            ("prompt_id", json!({"type": "string"})),
        );
    });
    assert!(breaks.is_empty(), "additions broke clients: {breaks:?}");
}

#[test]
fn removed_method_is_breaking() {
    let breaks = edited(|doc| {
        let methods = doc
            .get_mut("methods")
            .and_then(Value::as_array_mut)
            .expect("methods");
        methods.retain(|m| m.get("name").and_then(Value::as_str) != Some("terminal.kill"));
    });
    assert_eq!(rules(&breaks), ["method_removed"]);
    assert_eq!(
        breaks.first().map(|b| b.path.as_str()),
        Some("terminal.kill")
    );
}

#[test]
fn removed_required_output_is_breaking() {
    let breaks = edited(|doc| {
        let properties = at(doc, "/components/schemas/TerminalRecord/properties");
        properties
            .as_object_mut()
            .expect("properties")
            .remove("terminal_id");
    });
    assert!(
        rules(&breaks).contains(&"output_property_removed"),
        "{breaks:?}"
    );
    assert!(
        breaks
            .iter()
            .any(|b| b.path == "terminal.get ->.terminal_id"),
        "{breaks:?}"
    );
}

#[test]
fn optional_output_is_breaking_when_it_was_required() {
    let breaks = edited(|doc| {
        list(doc, "/components/schemas/TerminalRecord/required").retain(|k| k != "revision");
    });
    assert!(
        rules(&breaks).contains(&"output_no_longer_required"),
        "{breaks:?}"
    );
}

#[test]
fn newly_required_input_is_breaking() {
    let breaks = edited(|doc| {
        insert(
            doc,
            "/components/schemas/Expect",
            ("required", json!(["status_seq"])),
        );
    });
    assert!(
        rules(&breaks).contains(&"input_property_newly_required"),
        "{breaks:?}"
    );
    assert!(
        breaks
            .iter()
            .any(|b| b.path == "input.send(expect).status_seq"),
        "{breaks:?}"
    );
}

#[test]
fn narrowed_input_values_are_breaking() {
    let breaks = edited(|doc| {
        list(doc, "/components/schemas/TextFormat/oneOf").retain(|v| !is_const(v, "cells"));
    });
    assert!(
        rules(&breaks).contains(&"input_values_narrowed"),
        "{breaks:?}"
    );
}

#[test]
fn narrowed_input_type_is_breaking() {
    let breaks = edited(|doc| {
        *definition(doc, "TerminalId") = json!({"type": "integer"});
    });
    assert!(
        rules(&breaks).contains(&"input_type_narrowed"),
        "{breaks:?}"
    );
}

#[test]
fn removed_input_variant_is_breaking() {
    let breaks = edited(|doc| {
        list(doc, "/components/schemas/Target/oneOf").truncate(2);
    });
    assert!(
        rules(&breaks).contains(&"input_shape_removed"),
        "{breaks:?}"
    );
}

#[test]
fn closed_output_enum_growth_is_breaking() {
    let cursor = "/components/schemas/ScreenCursor/properties";
    let breaks = edited(|doc| {
        insert(
            doc,
            cursor,
            ("shape", json!({"type": "string", "enum": ["block"]})),
        );
    });
    assert!(breaks.is_empty(), "adding a field is additive: {breaks:?}");
    let mut grown = released().clone();
    insert(
        &mut grown,
        cursor,
        ("shape", json!({"type": "string", "enum": ["block"]})),
    );
    let mut later = grown.clone();
    insert(
        &mut later,
        cursor,
        ("shape", json!({"type": "string", "enum": ["block", "bar"]})),
    );
    let breaks = breaking_changes(&grown, &later).expect("well formed");
    assert!(
        rules(&breaks).contains(&"closed_output_enum_grew"),
        "{breaks:?}"
    );
}

#[test]
fn removed_event_is_breaking() {
    let breaks = edited(|doc| {
        let events = doc
            .get_mut("x-events")
            .and_then(Value::as_array_mut)
            .expect("events");
        events.retain(|e| e.get("name").and_then(Value::as_str) != Some("turn.ended"));
    });
    assert_eq!(rules(&breaks), ["event_removed"]);
}

#[test]
fn malformed_document_is_a_typed_failure() {
    let error = breaking_changes(released(), &json!({})).expect_err("no methods section");
    assert_eq!(error.kind(), "schema_malformed");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn every_removed_method_is_reported(mask in prop::collection::vec(any::<bool>(), all_methods().count())) {
        let names: Vec<&str> = all_methods().map(|spec| spec.name).collect();
        let removed: Vec<&str> = names.iter().zip(&mask).filter(|(_, drop)| **drop).map(|(n, _)| *n).collect();
        let breaks = edited(|doc| {
            let methods = doc.get_mut("methods").and_then(Value::as_array_mut).expect("methods");
            methods.retain(|m| !removed.iter().any(|r| m.get("name").and_then(Value::as_str) == Some(*r)));
        });
        let reported: Vec<&str> = breaks.iter().map(|b| b.path.as_str()).collect();
        prop_assert_eq!(reported, removed);
        prop_assert!(breaks.iter().all(|b| b.rule.token() == "method_removed"));
    }

    #[test]
    fn optional_params_never_break(index in 0..all_methods().count(), name in "[a-z]{3,12}") {
        let breaks = edited(|doc| {
            let params = list(doc, &format!("/methods/{index}/params"));
            params.push(json!({"name": format!("x_{name}"), "required": false, "schema": {"type": "string"}}));
        });
        prop_assert!(breaks.is_empty(), "{:?}", breaks);
    }
}
