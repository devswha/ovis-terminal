//! 1 MiB `doc.set` through the real socket, dispatcher, ledger and SQLite store, deduplicated
//! by params digest and bounded per method (plan todo 78). `happy` and `failure` are the QA
//! entry points named by the plan.
#![cfg(unix)]

#[path = "documents_retention/served.rs"]
pub mod served;
#[path = "../src/dispatch/qa/socket.rs"]
pub mod socket;

use ovis_terminal_core::ops::ParamsBound;
use serde_json::{Value, json};
use served::{Served, TestResult, call, get, op, set};
use socket::{at, error_kind};

/// A string whose compact JSON encoding is exactly 1 MiB, ending in `last`.
fn mebibyte(last: char) -> String {
    format!("{}{last}", "x".repeat((1 << 20) - 3))
}

#[tokio::test]
async fn happy() -> TestResult {
    // Given: a 1 MiB document value.
    let served = Served::start().await?;
    let mut client = served.client().await?;
    let value = mebibyte('a');
    assert_eq!(serde_json::to_vec(&value)?.len(), 1 << 20);
    let op_id = op(0)?;
    let write = set(
        &op_id,
        json!({"name": "big", "version": "0", "value": value}),
    );

    // When: it is written, read back, and the identical request is sent again.
    let committed = call(&mut client, &write).await?;
    let document = get(&mut client, "big").await?;
    let replayed = call(&mut client, &write).await?;

    // Then: version 1 is committed, read back byte-exact, and the retry replays the receipt.
    assert_eq!(at(&committed, "/result/version"), "1", "{committed}");
    assert_eq!(at(&document, "/version"), "1");
    let read = at(&document, "/value").as_str().ok_or("string value")?;
    assert!(
        read == value,
        "the 1 MiB value differs after the round trip"
    );
    assert_eq!(at(&replayed, "/result"), at(&committed, "/result"));
    assert_eq!(at(&get(&mut client, "big").await?, "/version"), "1");
    println!(
        "committed version={} value_bytes={} read_back_exact=true",
        at(&committed, "/result/version"),
        serde_json::to_vec(&value)?.len()
    );
    println!("identical retry replayed={}", at(&replayed, "/result"));
    drop(client);
    served.close().await
}

/// Characters of a value whose canonical `doc.set` params (op id removed, keys sorted,
/// `machine: null` included) are exactly `bytes` long at version "1".
fn chars_for_params(bytes: u64) -> TestResult<usize> {
    let skeleton = json!({"machine": null, "name": "big", "value": "", "version": "1"});
    let overhead = serde_json::to_vec(&skeleton)?.len();
    Ok(usize::try_from(bytes)?
        .checked_sub(overhead)
        .ok_or("bound below the skeleton")?)
}

#[tokio::test]
async fn failure() -> TestResult {
    // Given: "big" committed at version 1 with a 1 MiB value.
    let served = Served::start().await?;
    let mut client = served.client().await?;
    let original = mebibyte('a');
    let op_id = op(0)?;
    let first = json!({"name": "big", "version": "0", "value": original});
    call(&mut client, &set(&op_id, first)).await?;

    // When: the op id is retried with its last byte changed, a value one byte above the
    // document bound is written, params one byte above the doc.set bound are sent, and the
    // version is given as a JSON number.
    let changed = json!({"name": "big", "version": "0", "value": mebibyte('b')});
    let reused = call(&mut client, &set(&op_id, changed)).await?;
    let over_op = op(0)?;
    let over_value = format!("{}a", mebibyte('a'));
    let over_document = json!({"name": "big", "version": "1", "value": over_value});
    let over_document = call(&mut client, &set(&over_op, over_document)).await?;
    let bound = ParamsBound::Document.bytes().get();
    let beyond = "x".repeat(chars_for_params(bound + 1)?);
    let beyond = json!({"name": "big", "version": "1", "value": beyond});
    let over_params = call(&mut client, &set(&op(0)?, beyond)).await?;
    let numeric = json!({"name": "big", "version": 1, "value": "small"});
    let numeric = call(&mut client, &set(&op(0)?, numeric)).await?;

    // Then: op_reused names the original op; the document bound refuses inside the admitted
    // operation; the params bound refuses before admission with its hint; the numeric
    // counter is invalid_params; and the document is unchanged.
    assert_eq!(error_kind(&reused), "op_reused", "{reused}");
    assert_eq!(at(&reused, "/error/data/op_id"), &json!(op_id));
    assert_eq!(error_kind(&over_document), "too_large", "{over_document}");
    assert_eq!(at(&over_document, "/error/data/op_id"), &json!(over_op));
    assert_eq!(error_kind(&over_params), "too_large", "{over_params}");
    assert_eq!(at(&over_params, "/error/data/op_id"), &Value::Null);
    assert_eq!(
        at(&over_params, "/error/data/hint"),
        ParamsBound::Document.hint()
    );
    assert_eq!(error_kind(&numeric), "invalid_params", "{numeric}");
    let document = get(&mut client, "big").await?;
    assert_eq!(at(&document, "/version"), "1");
    assert!(
        at(&document, "/value").as_str() == Some(original.as_str()),
        "the stored 1 MiB value changed"
    );
    println!("changed 1 MiB retry -> {}", at(&reused, "/error/data"));
    println!(
        "1 MiB + 1 byte value -> {}",
        at(&over_document, "/error/data")
    );
    println!(
        "params of {} bytes (bound {bound}) -> {}",
        bound + 1,
        at(&over_params, "/error/data")
    );
    println!("numeric version -> {}", at(&numeric, "/error/data"));
    drop(client);
    served.close().await
}
