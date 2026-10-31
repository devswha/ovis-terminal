//! Exact persisted operation-entry boundary.

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};
use serde_json::json;

use super::super::Request;
use super::super::ledger::Entry;
use super::encode_entry;
use crate::registry::ServerBootId;

fn entry() -> Entry {
    let request = Request::new(
        OpId::new("000005f5-e100-7000-8000-000000000001".into()),
        "terminal.run".into(),
        json!({"argv":["bounded"]}),
    )
    .expect("valid request");
    let mut record = OpRecord::accepted(request.op_id.clone(), request.method);
    record.state = OpenEnum::Known(OpState::Committed);
    record.result = Some(json!({"payload":""}));
    Entry {
        principal: PrincipalId::parse("operator").expect("valid principal"),
        params: request.params,
        accepted_at: TimestampMs::new(100_000_000),
        boot: ServerBootId::try_from(1).expect("valid boot"),
        record,
    }
}

#[test]
fn persisted_receipt_accepts_16384_bytes_and_rejects_16385() {
    // Given a complete persisted entry adjusted to the exact receipt boundary.
    let mut accepted = entry();
    let base = serde_json::to_vec(&accepted).expect("entry encodes").len();
    let payload = "x".repeat((16 << 10) - base);
    accepted.record.result = Some(json!({"payload":payload}));
    assert_eq!(
        serde_json::to_vec(&accepted).expect("entry encodes").len(),
        16 << 10
    );

    // When the exact entry and a one-byte-larger entry cross production encoding.
    let encoded = encode_entry(&accepted).expect("exact boundary");
    let mut rejected = accepted;
    rejected.record.result = Some(json!({
        "payload":format!(
            "{}x",
            rejected.record.result
                .as_ref()
                .and_then(|result| result.get("payload"))
                .and_then(serde_json::Value::as_str)
                .expect("payload")
        )
    }));
    assert_eq!(
        serde_json::to_vec(&rejected).expect("entry encodes").len(),
        (16 << 10) + 1
    );
    let error = encode_entry(&rejected).expect_err("one byte over");

    // Then exactly 16 KiB is retained and 16 KiB plus one is typed too_large.
    assert_eq!(encoded.len(), 16 << 10);
    assert_eq!(error.kind(), "too_large");
}
