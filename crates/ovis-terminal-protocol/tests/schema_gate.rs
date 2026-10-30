//! Regressions for the gate review probes P1-P7 and the malformed-baseline CLI fixtures.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use ovis_terminal_protocol::errors::{ErrorData, ErrorKind};
use ovis_terminal_protocol::rpc::parse_request;
use ovis_terminal_protocol::schema::methods::input::InputSendParams;
use ovis_terminal_protocol::schema::{Break, breaking_changes, decode_params, openrpc_document};
use serde_json::{Value, json};

#[cfg(test)]
fn released() -> &'static Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(|| openrpc_document().expect("the catalog should generate"))
}

#[cfg(test)]
fn edit(base: &Value, pointer: &str, change: impl FnOnce(&mut Value)) -> Value {
    let mut document = base.clone();
    change(
        document
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("{pointer} exists")),
    );
    document
}

#[cfg(test)]
fn rules(released: &Value, current: &Value) -> Vec<(String, &'static str)> {
    let breaks: Vec<Break> = breaking_changes(released, current).expect("well formed");
    breaks
        .into_iter()
        .map(|b| (b.path, b.rule.token()))
        .collect()
}

#[cfg(test)]
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures")
        .join(name)
}

#[test]
fn p1_lowered_numeric_maximum_is_a_break() {
    let current = edit(
        released(),
        "/components/schemas/Geometry/properties/cols",
        |cols| {
            cols["maximum"] = json!(80);
        },
    );
    let found = rules(released(), &current);
    assert!(
        found.contains(&(
            "terminal.run(geometry).cols".to_owned(),
            "input_bound_narrowed"
        )),
        "{found:?}"
    );
}

#[test]
fn p1_raised_minimum_and_lowered_max_items_are_breaks() {
    let current = edit(
        released(),
        "/components/schemas/Geometry/properties/rows",
        |rows| {
            rows["minimum"] = json!(10);
        },
    );
    assert!(
        rules(released(), &current)
            .iter()
            .any(|(_, rule)| *rule == "input_bound_narrowed")
    );
    let current = edit(
        released(),
        "/components/schemas/EventFilter/properties/terminals",
        |terminals| {
            terminals["maxItems"] = json!(4);
        },
    );
    let found = rules(released(), &current);
    assert!(
        found.contains(&(
            "events.subscribe(filter).terminals".to_owned(),
            "input_bound_narrowed"
        )),
        "{found:?}"
    );
}

#[test]
fn p2_free_string_becoming_an_enum_is_a_break() {
    let current = edit(released(), "/components/schemas/TerminalId", |id| {
        id["enum"] = json!(["only-this-terminal"]);
    });
    assert!(
        rules(released(), &current)
            .iter()
            .any(|(_, rule)| *rule == "input_values_narrowed")
    );
}

#[test]
fn p2_tightened_pattern_is_a_break() {
    let current = edit(released(), "/components/schemas/TerminalId", |id| {
        id["pattern"] = json!("^t[0-9]+$");
    });
    assert!(
        rules(released(), &current)
            .iter()
            .any(|(_, rule)| *rule == "input_pattern_changed")
    );
}

#[test]
fn widened_bounds_stay_compatible() {
    let current = edit(
        released(),
        "/components/schemas/Geometry/properties/cols",
        |cols| {
            cols["minimum"] = json!(-1);
        },
    );
    assert_eq!(rules(released(), &current), Vec::new());
}

#[test]
fn p3_required_output_removed_inside_a_union_variant_is_a_break() {
    let union = json!({"oneOf": [
        {"type": "object", "properties": {"ready": {"type": "boolean"}, "terminal_id": {"type": "string"}}, "required": ["ready", "terminal_id"]},
        {"type": "object", "properties": {"ended": {"type": "boolean"}, "terminal_id": {"type": "string"}}, "required": ["ended", "terminal_id"]}
    ]});
    let base = edit(released(), "/methods/0/result/schema", |schema| {
        *schema = union;
    });
    let current = edit(
        &base,
        "/methods/0/result/schema/oneOf/0/required",
        |required| *required = json!(["ready"]),
    );
    assert!(rules(&base, &current).contains(&(
        "hello ->.terminal_id".to_owned(),
        "output_no_longer_required"
    )));
    let current = edit(&base, "/methods/0/result/schema/oneOf", |members| {
        members.as_array_mut().expect("members").truncate(1);
    });
    assert!(
        rules(&base, &current)
            .iter()
            .any(|(_, rule)| *rule == "output_variant_removed")
    );
}

#[test]
fn p4_every_published_error_kind_decodes_into_error_data_and_round_trips() {
    let kinds = released()
        .pointer("/x-error-kinds")
        .and_then(Value::as_array)
        .expect("kinds");
    let names = released()
        .pointer("/components/schemas/ErrorKindName/enum")
        .and_then(Value::as_array)
        .expect("enum");
    assert_eq!(kinds.len(), names.len());
    for token in names.iter().filter_map(Value::as_str) {
        let wire = json!({"kind": token, "hint": "recover", "retryable": false});
        let data: ErrorData =
            serde_json::from_value(wire.clone()).expect("a published kind decodes");
        assert_eq!(data.kind.token(), token);
        assert_eq!(serde_json::to_value(&data).expect("encodes"), wire);
    }
    let forbidden: ErrorData = serde_json::from_value(
        json!({"kind": "forbidden", "hint": "request authority", "retryable": false}),
    )
    .expect("decodes");
    assert_eq!(forbidden.kind, ErrorKind::Other("forbidden".to_owned()));
    let known: ErrorData =
        serde_json::from_value(json!({"kind": "not_found", "hint": "h", "retryable": false}))
            .expect("decodes");
    assert_eq!(known.kind, ErrorKind::NotFound);
}

#[test]
fn p5_missing_submit_reports_its_own_path() {
    let request = parse_request(r#"{"jsonrpc":"2.0","id":"probe","method":"input.send","params":{"op_id":"o1","target":{"terminal_id":"t1"},"text":"ls"}}"#).expect("envelope");
    let error = decode_params::<InputSendParams>(&request).expect_err("missing submit");
    assert_eq!(error.field_path(), "params.submit");
}

#[test]
fn p6_unnamed_released_method_is_a_typed_malformed_baseline() {
    let malformed = edit(released(), "/methods", |methods| {
        methods
            .as_array_mut()
            .expect("methods")
            .push(json!({"unknown_name": "retired.method"}));
    });
    let error =
        breaking_changes(&malformed, released()).expect_err("an unnamed entry is malformed");
    assert_eq!(error.kind(), "malformed_baseline");
    let extra_key = edit(released(), "/methods/0", |method| {
        method["surprise"] = json!(true);
    });
    let error = breaking_changes(&extra_key, released()).expect_err("an unknown key is malformed");
    assert_eq!(error.kind(), "malformed_baseline");
}

#[test]
fn p7_required_notification_output_removal_is_a_break() {
    let current = edit(released(), "/components/schemas/Heartbeat", |heartbeat| {
        heartbeat["properties"]
            .as_object_mut()
            .expect("properties")
            .remove("seq");
        heartbeat["required"]
            .as_array_mut()
            .expect("required")
            .retain(|field| field != "seq");
    });
    assert!(
        rules(released(), &current)
            .contains(&("heartbeat ->.seq".to_owned(), "output_property_removed"))
    );
    let current = edit(released(), "/x-notifications", |list| {
        list.as_array_mut()
            .expect("notifications")
            .retain(|n| n["name"] != "missed");
    });
    assert_eq!(
        rules(released(), &current),
        [("missed".to_owned(), "notification_removed")]
    );
}

#[test]
fn cli_compat_refuses_a_malformed_baseline_and_reports_a_retired_method() {
    let run = |name: &str| {
        Command::new(env!("CARGO_BIN_EXE_openrpc"))
            .arg("compat")
            .arg(fixture(name))
            .output()
            .expect("the CLI runs")
    };
    let malformed = run("malformed-released.json");
    assert_eq!(malformed.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&malformed.stderr).expect("stderr is JSON");
    assert_eq!(report.get("kind"), Some(&json!("malformed_baseline")));
    assert!(
        malformed.stdout.is_empty(),
        "no success line may be printed"
    );
    let retired = run("retired-method-released.json");
    assert_eq!(retired.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&retired.stderr).expect("stderr is JSON");
    assert_eq!(report.get("kind"), Some(&json!("schema_incompatible")));
    assert_eq!(
        report.get("breaks"),
        Some(&json!([{"path": "retired.method", "rule": "method_removed"}]))
    );
}
