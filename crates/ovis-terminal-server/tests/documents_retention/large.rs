//! Documents read back byte-exact up to the largest value `doc.set` accepts today.

use serde_json::{Value, json};

use super::socket::{at, error_kind};
use super::{Served, TestResult, call, get, op, set};

/// Characters of a string value whose canonical `doc.set` params (op id removed, keys
/// sorted, `machine: null` included) are exactly `bytes` long for a three-letter name.
fn value_chars_for_params(bytes: usize) -> TestResult<usize> {
    let skeleton = json!({"machine": null, "name": "big", "value": "", "version": "0"});
    Ok(bytes
        .checked_sub(serde_json::to_vec(&skeleton)?.len())
        .ok_or("params bound below the skeleton")?)
}

#[tokio::test]
async fn documents_read_back_byte_exact_up_to_the_largest_accepted_write() -> TestResult {
    // Given: the old 16 KiB receipt failure point and the largest value doc.set accepts
    // today, whose canonical params are exactly the 262144-byte ledger bound.
    let served = Served::start().await?;
    let mut client = served.client().await?;
    let largest = value_chars_for_params(262_144)?;
    assert_eq!(largest, 262_090, "encoded value of 262092 bytes");
    let canonical =
        json!({"machine": null, "name": "big", "value": "x".repeat(largest), "version": "0"});
    assert_eq!(serde_json::to_vec(&canonical)?.len(), 262_144);
    let sizes = [("edge", "e".repeat(16_382)), ("big", "x".repeat(largest))];

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

    // And: one more character exceeds the ledger bound before admission.
    let over = call(
        &mut client,
        &set(
            &op(0)?,
            json!({"name": "big", "version": "1", "value": "x".repeat(largest + 1)}),
        ),
    )
    .await?;
    assert_eq!(error_kind(&over), "too_large", "{over}");
    assert_eq!(at(&over, "/error/data/op_id"), &Value::Null);
    drop(client);
    served.close().await
}
