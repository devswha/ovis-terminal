//! Exact operation admission and parameter bounds.

use ovis_terminal_core::ops::{OperationError, PARAMS_ENVELOPE_BYTES, ParamsBound};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::limits::Limits;
use serde_json::{Value, json};

use super::{Fixture, Operations, Request, TestResult, context};

fn op_id(sequence: u128) -> OpId {
    let timestamp = 100_000_000_u128 << 80;
    let version = 7_u128 << 76;
    let variant = 2_u128 << 62;
    OpId::new(uuid::Uuid::from_u128(timestamp | version | variant | sequence).to_string())
}

fn capacity_request(sequence: u128) -> Result<Request, Box<dyn std::error::Error>> {
    Ok(Request::new(
        op_id(sequence),
        "terminal.run".into(),
        json!({"argv":[format!("op-{sequence}")]}),
    )?)
}

fn params_with_encoded_len(target: usize) -> Result<Value, Box<dyn std::error::Error>> {
    let empty = json!({"value":""});
    let overhead = serde_json::to_vec(&empty)?.len();
    let payload = target
        .checked_sub(overhead)
        .ok_or("target below overhead")?;
    Ok(json!({"value":"x".repeat(payload)}))
}

#[test]
fn capacity_accepts_1024_and_rejects_1025_without_eviction() -> TestResult {
    // Given a runtime filled to its advertised retained-operation capacity.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let first_request = capacity_request(1)?;
    let first = operations.accept(&first_request)?;
    for sequence in 2..=1024 {
        operations.accept(&capacity_request(sequence)?)?;
    }
    let last_request = capacity_request(1024)?;
    let last = operations.get(last_request.op_id())?;

    // When one additional operation is submitted.
    let overflow = capacity_request(1025)?;
    assert_eq!(
        operations
            .accept(&overflow)
            .expect_err("capacity boundary")
            .kind(),
        "backpressure"
    );
    assert_eq!(operations.get(first_request.op_id())?, first);
    assert_eq!(operations.get(last_request.op_id())?, last);
    assert_eq!(
        operations
            .get(overflow.op_id())
            .expect_err("never admitted")
            .kind(),
        "not_found"
    );
    core.close()?;

    // Then reopen preserves every retained receipt and the same admission boundary.
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    assert_eq!(operations.get(first_request.op_id())?, first);
    assert_eq!(
        operations
            .accept(&overflow)
            .expect_err("capacity survives reopen")
            .kind(),
        "backpressure"
    );
    core.close()?;
    Ok(())
}

fn assert_bound(method: &str, bound: ParamsBound) -> TestResult {
    // Given canonical parameter objects exactly at and one byte above the method's bound.
    let limit = usize::try_from(bound.bytes().get())?;
    let accepted_params = params_with_encoded_len(limit)?;
    let rejected_params = params_with_encoded_len(limit.checked_add(1).ok_or("limit")?)?;
    assert_eq!(serde_json::to_vec(&accepted_params)?.len(), limit);

    // When each object crosses Request's parse-once boundary.
    let accepted = Request::new(op_id(1), method.into(), accepted_params)?;
    let Err(rejected) = Request::new(op_id(2), method.into(), rejected_params) else {
        return Err("one byte over the bound was accepted".into());
    };

    // Then the exact boundary is usable and the next byte is too_large naming the bound.
    assert_eq!(accepted.op_id(), &op_id(1));
    assert_eq!(rejected.kind(), "too_large");
    assert_eq!(rejected.hint(), bound.hint());
    let OperationError::ParamsTooLarge {
        bound: named,
        actual,
    } = rejected
    else {
        return Err(format!("{rejected:?}").into());
    };
    assert_eq!(
        (named, actual.get()),
        (bound, bound.bytes().get().saturating_add(1))
    );
    Ok(())
}

#[test]
fn canonical_parameters_accept_262144_bytes_and_reject_262145() -> TestResult {
    assert_eq!(ParamsBound::of("terminal.run"), ParamsBound::Default);
    assert_eq!(ParamsBound::Default.bytes().get(), 262_144);
    assert_bound("terminal.run", ParamsBound::Default)
}

#[test]
fn doc_set_params_are_bounded_by_the_advertised_document_bound() -> TestResult {
    // The doc.set bound is derived from hello's doc_max_bytes plus the envelope allowance.
    let advertised = Limits::v1().doc_max_bytes.get();
    assert_eq!(ParamsBound::of("doc.set"), ParamsBound::Document);
    assert_eq!(
        ParamsBound::Document.bytes().get(),
        advertised + PARAMS_ENVELOPE_BYTES.get()
    );
    assert_bound("doc.set", ParamsBound::Document)
}
