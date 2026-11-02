//! Parse-once decoding of params: closed objects, typed rejections and exact counters.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::{RpcRequest, parse_request};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::input::InputSendParams;
use ovis_terminal_protocol::schema::methods::layout::LayoutEditParams;
use ovis_terminal_protocol::schema::methods::terminal::{TerminalGetParams, TerminalKillParams};
use ovis_terminal_protocol::schema::methods::terminal_read::{
    TerminalHistoryParams, TerminalSearchParams,
};
use ovis_terminal_protocol::schema::scalar::{LineNumber, Revision};
use proptest::prelude::*;
use serde_json::{Value, json};

#[cfg(test)]
fn request(method: &str, params: &Value) -> RpcRequest {
    let envelope = json!({ "jsonrpc": "2.0", "id": "r1", "method": method, "params": params });
    parse_request(&envelope.to_string()).expect("the envelope is valid")
}

#[cfg(test)]
fn rejected_path<P: serde::de::DeserializeOwned + std::fmt::Debug>(params: &Value) -> String {
    let error = decode_params::<P>(&request("m", params)).expect_err("params must be rejected");
    assert_eq!(error.kind(), "invalid_params");
    error.field_path().to_owned()
}

#[test]
fn explicit_target_decodes() {
    let params: TerminalGetParams = decode_params(&request(
        "terminal.get",
        &json!({"target": {"name": "build"}}),
    ))
    .expect("a named target decodes");
    assert!(matches!(params.target, Target::Name(name) if name.as_str() == "build"));
}

#[test]
fn target_naming_two_terminals_is_rejected() {
    let path =
        rejected_path::<TerminalGetParams>(&json!({"target": {"terminal_id": "t1", "name": "x"}}));
    assert_eq!(path, "params.target");
}

#[test]
fn wrong_type_names_the_field_without_serde_prose() {
    let error = decode_params::<TerminalHistoryParams>(&request(
        "terminal.history",
        &json!({"target": {"terminal_id": "t1"}, "limit": "many", "format": "text"}),
    ))
    .expect_err("a string limit is rejected");
    let wire = BoundaryError::from(error);
    let data = serde_json::to_value(wire.data()).expect("error data encodes");
    let expected = json!({
        "kind": "invalid_params",
        "hint": ErrorKind::InvalidParams.hint(),
        "retryable": false,
        "field_path": "params.limit",
    });
    assert_eq!(
        data, expected,
        "the wire data carries only structured fields"
    );
}

#[test]
fn numeric_counter_is_rejected_and_u64_max_string_round_trips() {
    let numeric =
        json!({"target": {"terminal_id": "t1"}, "from_line": 5, "limit": 10, "format": "text"});
    assert_eq!(
        rejected_path::<TerminalHistoryParams>(&numeric),
        "params.from_line"
    );
    let max = json!({"target": {"terminal_id": "t1"}, "from_line": "18446744073709551615", "limit": 10, "format": "text"});
    let params: TerminalHistoryParams =
        decode_params(&request("terminal.history", &max)).expect("decodes");
    assert_eq!(params.from_line.map(LineNumber::get), Some(u64::MAX));
    let encoded = serde_json::to_value(&params).expect("encodes");
    assert_eq!(
        encoded.get("from_line"),
        Some(&json!("18446744073709551615"))
    );
}

#[test]
fn missing_required_field_is_rejected_at_its_own_path() {
    let path = rejected_path::<InputSendParams>(
        &json!({"op_id": "o1", "target": {"terminal_id": "t1"}, "text": "ls"}),
    );
    assert_eq!(path, "params.submit");
}

#[test]
fn every_missing_required_field_names_itself() {
    let complete =
        json!({"op_id": "o1", "target": {"terminal_id": "t1"}, "text": "ls", "submit": "enter"});
    for field in ["op_id", "target", "text", "submit"] {
        let mut params = complete.clone();
        params.as_object_mut().expect("object").remove(field);
        assert_eq!(
            rejected_path::<InputSendParams>(&params),
            format!("params.{field}")
        );
    }
}

#[test]
fn missing_nested_field_names_its_full_path() {
    let params =
        json!({"op_id": "o1", "target": {"terminal_id": "t1"}, "escalation": {"hup_ms": "1"}});
    assert_eq!(
        rejected_path::<TerminalKillParams>(&params),
        "params.escalation.term_ms"
    );
}

#[test]
fn misspelled_target_is_named_instead_of_the_field_it_left_absent() {
    // The typo leaves the split without its pane; the typo itself is the offending path.
    let typo = json!({"op_id": "o1", "edit": {"op": "split_pane", "axis": "horizontal"},
        "pane_id_typo": "other-pane"});
    assert_eq!(
        rejected_path::<LayoutEditParams>(&typo),
        "params.pane_id_typo"
    );
    let incomplete = json!({"op_id": "o1", "edit": {"op": "split_pane", "axis": "horizontal"}});
    assert_eq!(
        rejected_path::<LayoutEditParams>(&incomplete),
        "params.edit.pane_id"
    );
}

proptest! {
    #[test]
    fn unknown_top_level_field_is_rejected_at_its_path(field in "[a-z][a-z_]{0,15}") {
        prop_assume!(field != "target");
        let params = json!({ "target": {"terminal_id": "t1"}, field.clone(): true });
        prop_assert_eq!(rejected_path::<TerminalGetParams>(&params), format!("params.{field}"));
    }

    #[test]
    fn unknown_target_field_is_rejected_at_its_path(field in "[a-z][a-z_]{0,15}") {
        prop_assume!(!["terminal_id", "address", "name"].contains(&field.as_str()));
        let params = json!({ "target": { field.clone(): "p1" } });
        prop_assert_eq!(rejected_path::<TerminalGetParams>(&params), format!("params.target.{field}"));
    }

    #[test]
    fn revisions_round_trip_exactly(revision in any::<u64>()) {
        let params = json!({"target": {"terminal_id": "t1"}, "pattern": {"text": "x"}, "revision": revision.to_string(), "limit": 1});
        let decoded: TerminalSearchParams = decode_params(&request("terminal.search", &params)).expect("decodes");
        prop_assert_eq!(decoded.revision.map(Revision::get), Some(revision));
        let encoded = serde_json::to_value(&decoded).expect("encodes");
        prop_assert_eq!(encoded.get("revision"), Some(&json!(revision.to_string())));
    }
}
