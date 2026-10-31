//! The released-baseline gate over the published error-kind catalog, driven by the
//! authz-kind fixtures in `schema/fixtures/`.

use std::path::PathBuf;
use std::process::Command;

use ovis_terminal_protocol::schema::{breaking_changes, check_compatibility, openrpc_document};
use serde_json::{Value, json};

#[cfg(test)]
fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("JSON")
}

#[cfg(test)]
fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

#[cfg(test)]
fn list_mut<'a>(document: &'a mut Value, pointer: &str) -> &'a mut Vec<Value> {
    document
        .pointer_mut(pointer)
        .and_then(Value::as_array_mut)
        .expect("a list")
}

#[cfg(test)]
/// The generated document without `kinds` in its catalog and its kind-name enum.
fn without(kinds: &[&str]) -> Value {
    let mut document = openrpc_document().expect("the catalog should generate");
    let gone = |token: Option<&str>| token.is_some_and(|token| kinds.contains(&token));
    list_mut(&mut document, "/x-error-kinds").retain(|entry| !gone(at(entry, "/kind").as_str()));
    list_mut(&mut document, "/components/schemas/ErrorKindName/enum")
        .retain(|name| !gone(name.as_str()));
    document
}

#[cfg(test)]
fn breaks(released: &Value, current: &Value) -> Value {
    let found = breaking_changes(released, current).expect("well formed");
    found
        .into_iter()
        .map(|found| json!({ "path": found.path, "rule": found.rule.token() }))
        .collect()
}

#[test]
fn the_authz_kinds_are_published_and_additive() {
    let fixture = fixture("authz-kinds-additive.json");
    let kinds: Vec<&str> = at(&fixture, "/released_without_kinds")
        .as_array()
        .expect("kinds")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(kinds.len(), 5);
    let current = openrpc_document().expect("the catalog should generate");
    let published: Vec<&str> = at(&current, "/x-error-kinds")
        .as_array()
        .expect("catalog")
        .iter()
        .filter_map(|entry| at(entry, "/kind").as_str())
        .collect();
    for kind in &kinds {
        assert!(published.contains(kind), "{kind} is published");
    }
    let released = without(&kinds);
    assert_eq!(&breaks(&released, &current), at(&fixture, "/expect/breaks"));
    check_compatibility(&released, &current).expect("adding the kinds is compatible");
}

#[test]
fn removing_any_authz_kind_is_rejected() {
    let fixture = fixture("authz-kind-removed.json");
    let released = openrpc_document().expect("the catalog should generate");
    let removals = at(&fixture, "/removals").as_array().expect("removals");
    assert_eq!(removals.len(), 5);
    for removal in removals {
        let kind = at(removal, "/current_without_kind")
            .as_str()
            .expect("a kind");
        let current = without(&[kind]);
        let expected = json!([{
            "path": at(removal, "/expect/path"),
            "rule": at(removal, "/expect/rule"),
        }]);
        assert_eq!(breaks(&released, &current), expected, "removing {kind}");
        assert!(check_compatibility(&released, &current).is_err());
    }
}

#[test]
fn cli_compat_reports_a_retired_error_kind() {
    let mut released = openrpc_document().expect("the catalog should generate");
    let retired = json!({ "kind": "retired_kind", "code": -32000, "description": "Gone." });
    list_mut(&mut released, "/x-error-kinds").push(retired);
    let path = std::env::temp_dir().join(format!("ovt13-kinds-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&released).expect("encodes")).expect("writes");
    let output = Command::new(env!("CARGO_BIN_EXE_openrpc"))
        .arg("compat")
        .arg(&path)
        .output()
        .expect("the CLI runs");
    std::fs::remove_file(&path).expect("removes the baseline");
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stderr).expect("stderr is JSON");
    assert_eq!(at(&report, "/kind"), "schema_incompatible");
    assert_eq!(
        at(&report, "/breaks"),
        &json!([{ "path": "x-error-kinds(retired_kind)", "rule": "error_kind_removed" }])
    );
}
