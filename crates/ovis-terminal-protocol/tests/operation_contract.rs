//! Additive cancellation results and supporting-evidence counter boundaries.

use ovis_terminal_protocol::rpc::parse_request;
use ovis_terminal_protocol::schema::methods::server::{CancelOutcome, OpCancelResult, OpRecord};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_protocol::schema::{check_compatibility, decode_params, openrpc_document};
use serde_json::{Value, json};

fn fixture() -> Result<Value, serde_json::Error> {
    serde_json::from_str(include_str!(
        "../../../schema/fixtures/operation-cancel-additive.json"
    ))
}

#[test]
fn cancel_outcome_preserves_released_record_fields_and_exact_evidence() {
    // Given the additive cancellation fixture and the released document.
    let wire = fixture().expect("operation fixture");
    let released: Value =
        serde_json::from_str(include_str!("../../../schema/released.openrpc.json"))
            .expect("released schema");

    // When current and previous record readers inspect the same cancellation.
    let current: OpCancelResult = serde_json::from_value(wire.clone()).expect("current result");
    let previous_record: OpRecord = serde_json::from_value(wire.clone()).expect("record result");

    // Then the result is additive and every maximum counter remains exact.
    assert_eq!(
        current.outcome,
        OpenEnum::Known(CancelOutcome::AlreadyCommitted)
    );
    assert_eq!(current.operation, previous_record);
    assert_eq!(current.operation.updated_seq.get(), u64::MAX);
    let write = current
        .operation
        .evidence
        .as_ref()
        .and_then(|evidence| evidence.write.as_ref())
        .expect("write evidence");
    assert_eq!(write.output_offset.get(), u64::MAX);
    assert_eq!(write.ack_epoch.get(), u64::MAX);
    assert_eq!(write.intent.sequence.get(), u64::MAX);
    assert_eq!(write.intent.grant_epoch.get(), u64::MAX);
    assert_eq!(write.intent.fence_epoch.get(), u64::MAX);
    assert_eq!(serde_json::to_value(current).expect("encode"), wire);
    check_compatibility(&released, &openrpc_document().expect("current schema")).expect("additive");
}

#[test]
fn later_cancel_outcomes_preserve_the_inspectable_record() {
    // Given an additive outcome token from a later same-generation server.
    let mut wire = fixture().expect("operation fixture");
    *wire.get_mut("outcome").expect("outcome") = json!("later_outcome");

    // When an older client decodes the cancellation result.
    let decoded: OpCancelResult = serde_json::from_value(wire.clone()).expect("open output");

    // Then the unfamiliar token and its full operation record remain available.
    assert_eq!(decoded.outcome, OpenEnum::Unknown("later_outcome".into()));
    assert_eq!(serde_json::to_value(decoded).expect("encode"), wire);
}

#[test]
fn numeric_evidence_counters_are_typed_invalid_params() {
    // Given each accepted counter location in the cancellation evidence.
    for pointer in [
        "/updated_seq",
        "/evidence/intent/sequence",
        "/evidence/intent/grant_epoch",
        "/evidence/intent/fence_epoch",
        "/evidence/intent/requested_bytes",
        "/evidence/write/written_bytes",
        "/evidence/write/output_offset",
        "/evidence/write/intent/sequence",
        "/evidence/write/ack_epoch",
    ] {
        let mut wire = fixture().expect("operation fixture");
        *wire.pointer_mut(pointer).expect("fixture counter") = json!(u64::MAX);

        // When the wire value is a number instead of a canonical decimal string.
        let envelope = json!({"jsonrpc":"2.0","id":"counter","method":"op.cancel","params":wire});
        let request = parse_request(&envelope.to_string()).expect("envelope");
        let error = decode_params::<OpCancelResult>(&request).expect_err("numeric counter");

        // Then decoding rejects it before dispatch, without rounding.
        assert_eq!(error.kind(), "invalid_params", "{pointer}");
    }
}
