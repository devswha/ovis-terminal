//! Documents read back byte-exact up to the advertised 1 MiB document bound.

use serde_json::json;

use super::socket::{at, error_kind};
use super::{Served, TestResult, call, get, op, set};

/// Characters of a string value whose compact JSON encoding is `bytes` long.
fn chars_encoding_to(bytes: usize) -> TestResult<usize> {
    Ok(bytes.checked_sub(2).ok_or("shorter than the quotes")?)
}

#[tokio::test]
async fn documents_read_back_byte_exact_up_to_the_largest_accepted_write() -> TestResult {
    // Given: the old 16 KiB receipt failure point, the old 256 KiB ledger params bound, and
    // a value encoding to exactly doc_max_bytes.
    let served = Served::start().await?;
    let mut client = served.client().await?;
    let largest = chars_encoding_to(1 << 20)?;
    let sizes = [
        ("edge", "e".repeat(16_382)),
        ("ledger", "l".repeat(262_090)),
        ("big", "x".repeat(largest)),
    ];

    for (name, text) in &sizes {
        // When: each is written at version 0, then read with a subscriber watching.
        let written = call(
            &mut client,
            &set(
                &op(0)?,
                json!({"name": name, "version": "0", "value": text}),
            ),
        )
        .await?;
        assert_eq!(at(&written, "/result/version"), "1", "{name}");
        let observer = served.core.subscribe(None)?;
        let document = get(&mut client, name).await?;

        // Then: the value is byte-exact at version 1 and the read published nothing.
        assert_eq!(at(&document, "/version"), "1", "{name}");
        let value = at(&document, "/value").as_str().ok_or("string value")?;
        assert_eq!(value.len(), text.len(), "{name}");
        assert!(
            value == text.as_str(),
            "{name} differs after the round trip"
        );
        assert!(
            observer.delivery.try_recv().is_err(),
            "doc.get must not publish events"
        );
    }

    // And: one more character is admitted by the ledger and refused by the document bound.
    let over_op = op(0)?;
    let over = call(
        &mut client,
        &set(
            &over_op,
            json!({"name": "big", "version": "1", "value": "x".repeat(largest + 1)}),
        ),
    )
    .await?;
    assert_eq!(error_kind(&over), "too_large", "{over}");
    assert_eq!(at(&over, "/error/data/op_id"), &json!(over_op));
    drop(client);
    served.close().await
}
