//! Behavior tests for the typed boundary error wire shape and JSON-RPC code mapping.

use std::error::Error as _;

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, JsonRpcCode, OpId, TargetId};
use ovis_terminal_protocol::rpc::{RequestId, RpcFailure, RpcReply, parse_request};
use serde_json::Value;

#[test]
fn malformed_json_preserves_source_without_serializing_parser_prose() {
    let error = parse_request("{").expect_err("malformed JSON must fail");
    let source = error
        .source()
        .expect("serde parser source must be preserved");
    let source_text = source.to_string();
    let reply = RpcReply::Failure(RpcFailure::new(
        RequestId::new("invalid-json".to_owned()),
        &error,
    ));
    let wire = serde_json::to_value(reply).expect("boundary error should serialize");
    let wire_text = wire.to_string();

    assert_eq!(error.kind(), "parse_error");
    assert_eq!(
        wire.pointer("/error/data/kind")
            .cloned()
            .unwrap_or(Value::Null),
        "parse_error"
    );
    assert!(
        !wire_text.contains(&source_text),
        "the internal parser diagnostic must not leak into wire JSON"
    );
}

#[test]
fn unknown_method_error_identifies_the_method_field() {
    let error = BoundaryError::method_not_found();
    let reply = RpcReply::Failure(RpcFailure::new(
        RequestId::new("unknown-method".to_owned()),
        &error,
    ));
    let wire = serde_json::to_value(reply).expect("boundary error should serialize");

    assert_eq!(error.kind(), "method_not_found");
    assert_eq!(error.field_path(), Some("method"));
    assert_eq!(
        wire.pointer("/error/data/field_path")
            .cloned()
            .unwrap_or(Value::Null),
        "method"
    );
}

#[test]
fn serializes_structured_boundary_error_fields() {
    let error = BoundaryError::invalid_params(Some("params.target.pane_id"))
        .with_target(TargetId::new("pane-1".to_owned()))
        .with_op_id(OpId::new("op-1".to_owned()));
    let display = error.to_string();
    let reply = RpcReply::Failure(RpcFailure::new(RequestId::new("r1".to_owned()), &error));
    let encoded = serde_json::to_value(reply).expect("error reply should serialize");

    assert_eq!(encoded.pointer("/id").cloned().unwrap_or(Value::Null), "r1");
    assert_eq!(
        encoded
            .pointer("/error/code")
            .cloned()
            .unwrap_or(Value::Null),
        -32602
    );
    assert_eq!(
        encoded
            .pointer("/error/data/kind")
            .cloned()
            .unwrap_or(Value::Null),
        "invalid_params"
    );
    assert_eq!(
        encoded
            .pointer("/error/data/hint")
            .cloned()
            .unwrap_or(Value::Null),
        ErrorKind::InvalidParams.hint()
    );
    assert_eq!(
        encoded
            .pointer("/error/message")
            .cloned()
            .unwrap_or(Value::Null),
        display
    );
    assert_eq!(
        encoded
            .pointer("/error/data/retryable")
            .cloned()
            .unwrap_or(Value::Null),
        false
    );
    assert_eq!(
        encoded
            .pointer("/error/data/field_path")
            .cloned()
            .unwrap_or(Value::Null),
        "params.target.pane_id"
    );
    assert_eq!(
        encoded
            .pointer("/error/data/target")
            .cloned()
            .unwrap_or(Value::Null),
        "pane-1"
    );
    assert_eq!(
        encoded
            .pointer("/error/data/op_id")
            .cloned()
            .unwrap_or(Value::Null),
        "op-1"
    );
}

#[test]
fn maps_typed_internal_error_to_json_rpc_internal_code() {
    let error = BoundaryError::domain(ErrorKind::Internal, false);

    assert_eq!(JsonRpcCode::from(&error), JsonRpcCode::InternalError);
    assert_eq!(JsonRpcCode::from(&error).value(), -32603);
    assert_eq!(error.kind(), "internal");
    assert_eq!(error.hint(), ErrorKind::Internal.hint());
}
