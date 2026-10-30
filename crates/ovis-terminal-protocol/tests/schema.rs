//! Behavior of the generated OpenRPC document and the released-baseline gate.

use std::path::PathBuf;

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, JsonRpcCode};
use ovis_terminal_protocol::rpc::parse_request;
use ovis_terminal_protocol::schema::methods::terminal::TerminalGetParams;
use ovis_terminal_protocol::schema::{
    SchemaError, check_compatibility, decode_params, openrpc_document, render_openrpc,
    verify_document,
};
use serde_json::Value;

#[cfg(test)]
fn repo_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema")
        .join(name)
}

#[cfg(test)]
fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(repo_file(name)).expect("schema file should be readable");
    serde_json::from_str(&text).expect("schema file should be JSON")
}

#[cfg(test)]
fn method_names(document: &Value) -> Vec<&str> {
    let methods = document.get("methods").and_then(Value::as_array);
    methods
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("name")?.as_str())
        .collect()
}

#[test]
fn happy() {
    let first = render_openrpc().expect("the catalog should generate");
    let second = render_openrpc().expect("the catalog should generate again");
    assert_eq!(first, second, "regeneration must be byte-stable");
    let checked_in = std::fs::read_to_string(repo_file("ovis-terminal.openrpc.json"))
        .expect("the generated document should be checked in");
    assert_eq!(
        first, checked_in,
        "schema/ovis-terminal.openrpc.json is stale"
    );

    let document: Value = serde_json::from_str(&first).expect("generated text should be JSON");
    verify_document(&document).expect("names should be unique and references resolve");
    let names = method_names(&document);
    for required in [
        "hello",
        "terminal.run",
        "input.send",
        "attach.open",
        "events.subscribe",
        "agent.register",
        "layout.edit",
        "selection.set",
        "worktree.remove",
        "plugin.invoke",
        "update.activate",
        "service.install",
        "machine.add",
        "rpc.discover",
    ] {
        assert!(names.contains(&required), "catalog lacks {required}");
    }
    let kinds = document
        .pointer("/x-stream/kinds")
        .and_then(Value::as_array);
    let kinds: Vec<&str> = kinds
        .into_iter()
        .flatten()
        .filter_map(|k| k.get("name")?.as_str())
        .collect();
    assert_eq!(kinds, ["DATA", "CREDIT", "CLOSE"]);
    println!(
        "methods={} events={} schemas={} bytes={}",
        names.len(),
        document
            .get("x-events")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        document
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .map_or(0, serde_json::Map::len),
        first.len()
    );

    check_compatibility(&load("released.openrpc.json"), &document)
        .expect("the current document should keep every released contract");
}

#[test]
fn failure() {
    let fixture = load("fixtures/breaking-required-param.json");
    let method = fixture
        .get("method")
        .and_then(Value::as_str)
        .expect("fixture names a method");
    let param = fixture
        .get("inject_param")
        .cloned()
        .expect("fixture carries a param");
    let released = load("released.openrpc.json");
    let mut current = openrpc_document().expect("the catalog should generate");
    let methods = current
        .get_mut("methods")
        .and_then(Value::as_array_mut)
        .expect("methods");
    let target = methods
        .iter_mut()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(method))
        .and_then(|m| m.get_mut("params"))
        .and_then(Value::as_array_mut)
        .expect("fixture method should exist with params");
    target.push(param);

    let error =
        check_compatibility(&released, &current).expect_err("a new required input must break");
    assert_eq!(
        error.kind(),
        fixture
            .pointer("/expect/kind")
            .and_then(Value::as_str)
            .expect("kind")
    );
    let SchemaError::Incompatible { breaks } = &error else {
        panic!("expected incompatible, got {error:?}")
    };
    let rule = fixture
        .pointer("/expect/rule")
        .and_then(Value::as_str)
        .expect("rule");
    let path = fixture
        .pointer("/expect/path")
        .and_then(Value::as_str)
        .expect("path");
    assert!(
        breaks
            .iter()
            .any(|b| b.rule.token() == rule && b.path == path),
        "breaks: {breaks:?}"
    );
    println!("injected {path}: {} -> {rule}", error.kind());

    let request = parse_request(
        r#"{"jsonrpc":"2.0","id":"r1","method":"terminal.get","params":{"target":{"pane_id":"p1"}}}"#,
    )
    .expect("the envelope itself is valid");
    let mut dispatched = 0_u32;
    let outcome = decode_params::<TerminalGetParams>(&request).map(|_| dispatched += 1);
    let rejection = outcome.expect_err("an unknown target property must not decode");
    assert_eq!(rejection.kind(), "invalid_params");
    assert_eq!(rejection.field_path(), "params.target.pane_id");
    assert_eq!(
        dispatched, 0,
        "nothing may be dispatched after a rejected decode"
    );
    let wire = BoundaryError::from(rejection);
    assert_eq!(wire.error_kind(), &ErrorKind::InvalidParams);
    assert_eq!(JsonRpcCode::from(&wire).value(), -32602);
    assert_eq!(wire.field_path(), Some("params.target.pane_id"));
    println!(
        "unknown target property: {} at {:?}",
        wire.kind(),
        wire.field_path()
    );
}

#[test]
fn released_baseline_is_a_valid_document() {
    verify_document(&load("released.openrpc.json")).expect("the baseline should verify");
}

#[test]
fn dangling_reference_is_rejected() {
    let mut document = openrpc_document().expect("the catalog should generate");
    let schemas = document
        .pointer_mut("/components/schemas")
        .and_then(Value::as_object_mut);
    schemas.expect("components").remove("Target");
    let error = verify_document(&document).expect_err("a missing definition must be reported");
    assert_eq!(error.kind(), "schema_unresolved_reference");
}

#[test]
fn duplicated_method_name_is_rejected() {
    let mut document = openrpc_document().expect("the catalog should generate");
    let methods = document
        .get_mut("methods")
        .and_then(Value::as_array_mut)
        .expect("methods");
    let copy = methods.first().cloned().expect("at least one method");
    methods.push(copy);
    let error = verify_document(&document).expect_err("a duplicate name must be reported");
    assert_eq!(error.kind(), "schema_duplicate_name");
}

#[test]
fn colliding_definition_name_is_rejected() {
    let mut document = openrpc_document().expect("the catalog should generate");
    let schemas = document
        .pointer_mut("/components/schemas")
        .and_then(Value::as_object_mut);
    schemas
        .expect("components")
        .insert("Target2".to_owned(), Value::Bool(true));
    let error = verify_document(&document).expect_err("a disambiguated name means a collision");
    assert_eq!(error.kind(), "schema_name_collision");
}
