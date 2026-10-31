//! Size and name bounds, and the version model under arbitrary write sequences.

use ovis_terminal_protocol::schema::methods::doc::DocSetParams;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::StoreOptions;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::fixture::{Root, TestResult, at, failed_kind, get, op_minted, set};
use crate::docs::{DOC_MAX_BYTES, DocWrite};

/// A string value whose compact JSON encoding is exactly `bytes` long (two quotes included).
fn encoded_as(bytes: u64) -> TestResult<Value> {
    let chars = usize::try_from(bytes.checked_sub(2).ok_or("too short")?)?;
    Ok(Value::String("x".repeat(chars)))
}

#[test]
fn the_document_bound_is_inclusive_at_exactly_one_mebibyte() -> TestResult {
    // Given: values encoding to exactly the advertised bound and one byte above it.
    let limit = DOC_MAX_BYTES.get();
    assert_eq!(limit, 1_048_576);
    let params = |value: Value| -> TestResult<DocSetParams> {
        let op_id = serde_json::to_value(op_minted(0)?)?;
        Ok(serde_json::from_value(
            json!({"op_id": op_id, "name": "big", "version": "0", "value": value}),
        )?)
    };

    // When: each is validated as a write.
    let fits = DocWrite::new(params(encoded_as(limit)?)?);
    let over = DocWrite::new(params(encoded_as(limit + 1)?)?);

    // Then: the bound itself is accepted and one byte more is too_large.
    assert!(fits.is_ok(), "{fits:?}");
    assert_eq!(over.err().map(|error| error.kind()), Some("too_large"));
    Ok(())
}

#[test]
fn one_mebibyte_plus_one_byte_is_too_large_and_leaves_the_version_unchanged() -> TestResult {
    // Given: a document committed at version 1.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    set(
        &core,
        &op_minted(0)?,
        json!({"name": "big", "version": "0", "value": "small"}),
    )??;

    // When: a 1 MiB + 1 byte value is written at the current version.
    let value = encoded_as(DOC_MAX_BYTES.get() + 1)?;
    let over = set(
        &core,
        &op_minted(0)?,
        json!({"name": "big", "version": "1", "value": value}),
    )?;

    // Then: it is refused as too_large and the stored document is untouched.
    assert_eq!(over.err().map(|error| error.kind()), Some("too_large"));
    let document = get(&core, "big")?;
    assert_eq!(
        (document.version.get(), document.value),
        (1, json!("small"))
    );
    Ok(core.close()?)
}

#[test]
fn malformed_names_fail_typed_without_writing() -> TestResult {
    // Given: names that are empty, too long, or use a separator byte.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    for name in [
        String::new(),
        "n".repeat(129),
        "a/b".to_owned(),
        "a b".to_owned(),
    ] {
        // When: each is written.
        let record = set(
            &core,
            &op_minted(0)?,
            json!({"name": name, "version": "0", "value": 1}),
        )??;

        // Then: invalid_name is retained and the 128-byte boundary is accepted.
        assert_eq!(
            failed_kind(&record).as_deref(),
            Some("invalid_name"),
            "{name:?}"
        );
    }
    let longest = set(
        &core,
        &op_minted(0)?,
        json!({"name": "n".repeat(128), "version": "0", "value": 1}),
    )??;
    assert_eq!(longest.state, OpenEnum::Known(OpState::Committed));
    Ok(core.close()?)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    #[test]
    fn versions_advance_by_one_per_commit_and_conflicts_change_nothing(
        writes in proptest::collection::vec((0_u64..4, any::<Option<u8>>()), 1..6)
    ) {
        let check = || -> TestResult {
            // Given: an empty store and a model of the document.
            let root = Root::new()?;
            let core = root.open(StoreOptions::default())?;
            let (mut version, mut value) = (0_u64, Value::Null);
            for (read, written) in writes.clone() {
                // When: a writer that read `read` stores `written` (None deletes).
                let next = written.map_or(Value::Null, |byte| json!(byte));
                let params = json!({"name": "model", "version": read.to_string(), "value": next});
                let record = set(&core, &op_minted(0)?, params)??;

                // Then: only a matching version commits, exactly one step ahead.
                if read == version {
                    version = version.checked_add(1).ok_or("version")?;
                    value = next;
                    assert_eq!(at(&record.result.ok_or("result")?, "/version"), &json!(version.to_string()));
                } else {
                    assert_eq!(failed_kind(&record).as_deref(), Some("conflict"));
                }
                let document = get(&core, "model")?;
                assert_eq!((document.version.get(), &document.value), (version, &value));
            }
            Ok(core.close()?)
        };
        check().map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}
