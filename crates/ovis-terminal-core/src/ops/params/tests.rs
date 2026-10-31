//! Params digest encoding and rows written before digests named their algorithm.

use proptest::prelude::*;
use serde_json::json;

use super::{DigestAlgorithm, Params, ParamsDigest};

mod legacy;

#[test]
fn a_digest_is_stored_with_its_algorithm_name() -> Result<(), serde_json::Error> {
    // Given: the digest of some canonical bytes.
    let digest = ParamsDigest::sha256(b"{}");

    // When: it is encoded as a ledger field.
    let stored = serde_json::to_value(&digest)?;

    // Then: the algorithm is named beside 64 lowercase hex digits.
    assert_eq!(stored.get("algorithm"), Some(&json!("sha-256")));
    let hex = stored.get("hex").and_then(serde_json::Value::as_str);
    assert_eq!(hex.map(str::len), Some(64));
    assert_eq!(serde_json::from_value::<ParamsDigest>(stored)?, digest);
    Ok(())
}

#[test]
fn digests_without_a_known_algorithm_or_with_bad_hex_are_refused() {
    // Given: stored digests that drop or misname the algorithm, or carry malformed hex.
    let good = "ab".repeat(32);
    let refused = [
        json!({"hex": good}),
        json!({"algorithm": "md5", "hex": good}),
        json!({"algorithm": "sha-256", "hex": "AB".repeat(32)}),
        json!({"algorithm": "sha-256", "hex": "ab".repeat(31)}),
        json!([1, 2, 3]),
    ];

    // When/Then: none decodes as a digest.
    for stored in refused {
        assert!(
            serde_json::from_value::<ParamsDigest>(stored.clone()).is_err(),
            "{stored}"
        );
    }
}

#[test]
fn a_row_without_any_digest_or_with_a_short_bare_digest_is_refused() {
    // Given: row params that name no digest, or a pre-78 digest of 31 bytes.
    for stored in [json!({}), json!({"params": vec![7_u8; 31]})] {
        // When/Then: the row's params do not decode, so no retry can match them.
        assert!(
            serde_json::from_value::<Params>(stored.clone()).is_err(),
            "{stored}"
        );
    }
}

proptest! {
    #[test]
    fn every_digest_round_trips_and_pre_78_bytes_read_as_sha_256(bytes in any::<[u8; 32]>()) {
        // Given: any 32 digest bytes, as a pre-78 row stored them.
        let legacy: Params = serde_json::from_value(json!({"params": bytes.to_vec()}))
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // When: the row is rewritten and read back.
        let rewritten = serde_json::to_value(&legacy)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let reread: Params = serde_json::from_value(rewritten.clone())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // Then: the bare field is gone, the algorithm is named and the digest is unchanged.
        prop_assert!(rewritten.get("params").is_none());
        prop_assert_eq!(&reread.digest, &legacy.digest);
        prop_assert_eq!(reread.digest.algorithm(), DigestAlgorithm::Sha256);
        let hex = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().concat();
        prop_assert_eq!(reread.digest.hex(), hex);
    }
}
