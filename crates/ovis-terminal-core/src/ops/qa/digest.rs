//! Reuse is decided by the params digest; the stored copy is bounded and says when it is cut.

use ovis_terminal_core::ops::{DigestAlgorithm, PARAMS_COPY_BYTES, ParamsView};
use ovis_terminal_protocol::errors::OpId;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{Fixture, Operations, Request, TestResult, context};

fn op_id(sequence: u128) -> OpId {
    let timestamp = 100_000_000_u128 << 80;
    let version = 7_u128 << 76;
    let variant = 2_u128 << 62;
    OpId::new(uuid::Uuid::from_u128(timestamp | version | variant | sequence).to_string())
}

/// `doc.set` params whose canonical encoding is exactly `bytes` long, ending in `last`.
fn document(bytes: usize, last: char) -> Result<Value, Box<dyn std::error::Error>> {
    let skeleton = serde_json::to_vec(&json!({"name": "big", "value": ""}))?.len();
    let chars = bytes
        .checked_sub(skeleton.saturating_add(last.len_utf8()))
        .ok_or("below skeleton")?;
    let value = format!("{}{last}", "x".repeat(chars));
    let params = json!({"name": "big", "value": value});
    assert_eq!(serde_json::to_vec(&params)?.len(), bytes);
    Ok(params)
}

fn copy_of(view: &ParamsView) -> Result<(&[u8], u64, bool), &'static str> {
    let copy = view.copy.as_ref().ok_or("copy retained")?;
    Ok((&copy.prefix, copy.canonical_bytes.get(), copy.truncated))
}

#[test]
fn a_one_byte_change_past_the_stored_copy_is_refused_by_digest() -> TestResult {
    // Given: a 1 MiB doc.set accepted; its stored copy holds only the first 256 KiB.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let original = document(1 << 20, 'a')?;
    let changed = document(1 << 20, 'b')?;
    let first = Request::new(op_id(1), "doc.set".into(), original.clone())?;
    let accepted = operations.accept(&first)?;

    // When: the same identity is retried identically, then with its last byte changed.
    let identical =
        operations.accept(&Request::new(op_id(1), "doc.set".into(), original.clone())?)?;
    let refused = operations
        .accept(&Request::new(op_id(1), "doc.set".into(), changed.clone())?)
        .expect_err("changed params");

    // Then: the identical retry replays the receipt and the change is op_reused, although
    // both canonical encodings agree on every byte the stored copy holds.
    assert_eq!(identical, accepted);
    assert_eq!(refused.kind(), "op_reused");
    assert_eq!(refused.receipt(), Some(&accepted));
    let view = operations.params(first.op_id())?;
    let (prefix, total, truncated) = copy_of(&view)?;
    let changed_bytes = serde_json::to_vec(&changed)?;
    assert_eq!(
        prefix,
        changed_bytes.get(..PARAMS_COPY_BYTES).ok_or("prefix")?
    );
    assert_eq!((total, truncated), (1 << 20, true));
    let original_bytes = serde_json::to_vec(&original)?;
    assert_eq!(view.digest.algorithm(), DigestAlgorithm::Sha256);
    assert_eq!(
        view.digest.hex(),
        format!("{:x}", Sha256::digest(&original_bytes))
    );
    core.close()?;
    Ok(())
}

#[test]
fn the_stored_copy_is_whole_up_to_256_kib_and_truncated_after() -> TestResult {
    // Given: canonical params of 262144 and 262145 bytes.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let cases = [
        (1, PARAMS_COPY_BYTES, false),
        (2, PARAMS_COPY_BYTES + 1, true),
    ];
    for (sequence, bytes, expected) in cases {
        let params = document(bytes, 'z')?;
        let request = Request::new(op_id(sequence), "doc.set".into(), params.clone())?;

        // When: each is accepted and inspected.
        operations.accept(&request)?;
        let view = operations.params(request.op_id())?;

        // Then: the copy is the whole encoding at the bound and a 256 KiB prefix past it.
        let (prefix, total, truncated) = copy_of(&view)?;
        let canonical = serde_json::to_vec(&params)?;
        assert_eq!(prefix, canonical.get(..PARAMS_COPY_BYTES).ok_or("prefix")?);
        assert_eq!(total, u64::try_from(bytes)?);
        assert_eq!(truncated, expected, "{bytes}");
    }
    core.close()?;
    Ok(())
}

#[test]
fn the_copy_is_cut_before_a_character_that_straddles_256_kib() -> TestResult {
    // Given: params whose two-byte final character starts at byte 262143.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let params = document(PARAMS_COPY_BYTES + 3, 'é')?;
    let canonical = serde_json::to_vec(&params)?;
    assert_eq!(
        canonical.get(PARAMS_COPY_BYTES - 1..PARAMS_COPY_BYTES + 1),
        Some("é".as_bytes())
    );
    let request = Request::new(op_id(1), "doc.set".into(), params)?;

    // When: it is accepted and inspected.
    operations.accept(&request)?;
    let view = operations.params(request.op_id())?;

    // Then: the copy stops before that character and stays valid UTF-8.
    let (prefix, _, truncated) = copy_of(&view)?;
    assert_eq!(prefix.len(), PARAMS_COPY_BYTES - 1);
    assert!(truncated);
    assert!(std::str::from_utf8(prefix).is_ok());
    core.close()?;
    Ok(())
}
