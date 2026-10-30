//! Behavior tests for correlated JSON-RPC request and response envelopes.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, JsonRpcCode};
use ovis_terminal_protocol::rpc::{
    InFlightRequests, RequestId, RpcReply, RpcSuccess, parse_reply, parse_request,
};
use serde_json::{Value, json};

#[test]
fn happy() {
    let first = parse_request(
        r#"{"jsonrpc":"2.0","id":"request-a","method":"terminal.get","params":{"target":{"terminal_id":"p1"}}}"#,
    )
    .expect("string ID request should parse");
    let second =
        parse_request(r#"{"jsonrpc":"2.0","id":"request-b","method":"terminal.list","params":{}}"#)
            .expect("second request should parse");

    let mut in_flight = InFlightRequests::default();
    in_flight
        .begin(first.id().clone())
        .expect("first ID should be accepted");
    in_flight
        .begin(second.id().clone())
        .expect("second ID should be accepted");

    let replies = [
        RpcReply::Success(RpcSuccess::new(
            first.id().clone(),
            json!({"pane_id": "p1"}),
        )),
        RpcReply::Success(RpcSuccess::new(second.id().clone(), json!({"panes": []}))),
    ];
    let encoded: Vec<Value> = replies
        .iter()
        .map(|reply| serde_json::to_value(reply).expect("reply should serialize"))
        .collect();

    assert_eq!(
        encoded
            .first()
            .expect("reply 0 should exist")
            .pointer("/id")
            .cloned()
            .unwrap_or(Value::Null),
        "request-a"
    );
    assert_eq!(
        encoded
            .first()
            .expect("reply 0 should exist")
            .pointer("/result/pane_id")
            .cloned()
            .unwrap_or(Value::Null),
        "p1"
    );
    assert!(
        encoded
            .first()
            .expect("reply 0 should exist")
            .get("error")
            .is_none()
    );
    assert_eq!(
        encoded
            .get(1)
            .expect("reply 1 should exist")
            .pointer("/id")
            .cloned()
            .unwrap_or(Value::Null),
        "request-b"
    );
    assert_eq!(
        encoded
            .get(1)
            .expect("reply 1 should exist")
            .pointer("/result/panes")
            .cloned()
            .unwrap_or(Value::Null),
        json!([])
    );
    assert!(
        encoded
            .get(1)
            .expect("reply 1 should exist")
            .get("error")
            .is_none()
    );
}

#[test]
fn failure() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "terminal.get",
        "params": {"pane_id": "p1"}
    });

    let error =
        parse_request(&input.to_string()).expect_err("numeric ID must be rejected before dispatch");
    assert_eq!(error.error_kind(), ErrorKind::InvalidRequest);
    assert_eq!(error.json_rpc_code(), JsonRpcCode::InvalidRequest);
    assert_eq!(error.field_path(), Some("id"));
}

#[test]
fn rejects_unknown_request_envelope_fields() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "r1",
        "method": "terminal.get",
        "params": {"target": {"terminal_id": "p1"}},
        "debug": true
    });

    let error = parse_request(&request.to_string())
        .expect_err("unknown envelope property must be rejected");

    assert_eq!(error.error_kind(), ErrorKind::InvalidRequest);
    assert_eq!(error.field_path(), Some("debug"));
}

#[test]
fn rejects_unknown_target_parameter_before_dispatch() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "r1",
        "method": "terminal.get",
        "params": {"pane_id": "p1"}
    });
    let parsed = parse_request(&request.to_string()).expect("request envelope should parse");
    let error = parsed
        .validate_params(&["target"])
        .expect_err("unknown target property must fail before dispatch");

    assert_eq!(error.error_kind(), ErrorKind::InvalidParams);
    assert_eq!(error.json_rpc_code(), JsonRpcCode::InvalidParams);
    assert_eq!(error.field_path(), Some("params.pane_id"));
}

#[test]
fn rejects_unknown_nested_target_field_before_dispatch() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "r1",
        "method": "terminal.get",
        "params": {"target": {"pane_id": "p1"}}
    });
    let parsed = parse_request(&request.to_string()).expect("request envelope should parse");
    let error = parsed
        .validate_params(&["target"])
        .expect_err("unknown nested target property must fail before dispatch");

    assert_eq!(error.error_kind(), ErrorKind::InvalidParams);
    assert_eq!(error.field_path(), Some("params.target.pane_id"));
}

#[test]
fn rejects_non_string_target_identity_before_dispatch() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "r1",
        "method": "terminal.get",
        "params": {"target": {"terminal_id": 17}}
    });
    let parsed = parse_request(&request.to_string()).expect("request envelope should parse");
    let error = parsed
        .validate_params(&["target"])
        .expect_err("target identity must be a string before dispatch");

    assert_eq!(error.error_kind(), ErrorKind::InvalidParams);
    assert_eq!(error.field_path(), Some("params.target.terminal_id"));
}

#[test]
fn rejects_error_and_result_in_one_response() {
    let response = json!({
        "jsonrpc": "2.0",
        "id": "r1",
        "result": {},
        "error": {"code": -32600, "message": "invalid", "data": {}}
    });

    let error = parse_reply(&response.to_string())
        .expect_err("a terminal response cannot contain both result and error");

    assert_eq!(error.error_kind(), ErrorKind::InvalidResponse);
    assert_eq!(error.json_rpc_code(), JsonRpcCode::InvalidRequest);
}

#[test]
fn rejects_response_with_wrong_json_rpc_version() {
    let response = json!({
        "jsonrpc": "1.0",
        "id": "r1",
        "result": {}
    });
    let error =
        parse_reply(&response.to_string()).expect_err("only JSON-RPC 2.0 responses are accepted");

    assert_eq!(error.error_kind(), ErrorKind::InvalidResponse);
}

#[test]
fn rejects_duplicate_in_flight_request_ids() {
    let id = RequestId::new("same-id".to_owned());
    let mut in_flight = InFlightRequests::default();

    in_flight
        .begin(id.clone())
        .expect("initial request should be accepted");
    assert_eq!(
        in_flight.begin(id),
        Err(BoundaryError::duplicate_request_id())
    );
}
