use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, params, request};

pub(crate) async fn wrong_target_and_committed_cancel() -> TestResult {
    // Given an authenticated socket whose only mutation is explicitly registered.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, std::sync::Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    let hello = client.hello("1", &fixture.socket.bearer).await?;
    assert!(
        !at(&hello, "/result/methods")
            .as_array()
            .ok_or("methods")?
            .contains(&json!("pane.split"))
    );
    // When legacy and misspelled targets are submitted.
    client
        .send(&json!({"jsonrpc":"2.0","id":"bad","method":"pane.split",
        "params":{"pane_id":"p2","op_id":"01800000-0000-7000-8000-000000000001"}}))
        .await?;
    let rejected = client.expect_next().await?;
    assert_eq!(error_kind(&rejected), "method_not_found");
    assert_eq!(at(&rejected, "/error/data/field_path"), "method");
    let op = op_id();
    let mut wrong = params(&op);
    wrong
        .as_object_mut()
        .ok_or("params object")?
        .insert("pane_id".into(), json!("p2"));
    client.send(&request("typo", "doc.set", &wrong)).await?;
    let rejected = client.expect_next().await?;
    assert_eq!(error_kind(&rejected), "invalid_params");
    assert_eq!(at(&rejected, "/error/data/field_path"), "params.pane_id");
    assert_eq!(*fixture.gate.entered.borrow(), 0);
    assert!(!fixture.socket.path("effect").exists());
    // Then no operation was admitted, while cancellation of committed work retains its receipt.
    client
        .send(&request("absent", "op.get", &json!({"op_id": op})))
        .await?;
    assert_eq!(error_kind(&client.expect_next().await?), "not_found");
    fixture.gate.release.notify_one();
    client
        .send(&request("commit", "doc.set", &params(&op)))
        .await?;
    let committed = client.expect_next().await?;
    assert_eq!(at(&committed, "/result/value"), "payload");
    assert_eq!(
        std::fs::read(fixture.socket.path("effect"))?,
        b"\"payload\""
    );
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id": op})))
        .await?;
    let cancelled = client.expect_next().await?;
    assert_eq!(at(&cancelled, "/result/outcome"), "already_committed");
    assert_eq!(at(&cancelled, "/result/result"), at(&committed, "/result"));
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}

#[tokio::test]
async fn malformed_counters_and_operation_ids_never_enter_the_handler() -> TestResult {
    // Given a real authenticated dispatch boundary.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, std::sync::Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    // When counters use numeric/noncanonical encoding or IDs use the wrong UUID version.
    for version in [
        json!(0),
        json!("00"),
        json!("-1"),
        json!("18446744073709551616"),
    ] {
        let mut invalid = params(&op_id());
        *invalid.get_mut("version").ok_or("version")? = version;
        client
            .send(&request("counter", "doc.set", &invalid))
            .await?;
        let rejected = client.expect_next().await?;
        assert_eq!(error_kind(&rejected), "invalid_params");
        assert_eq!(at(&rejected, "/error/data/field_path"), "params.version");
    }
    client
        .send(&request(
            "uuid",
            "doc.set",
            &params("01800000-0000-4000-8000-000000000001"),
        ))
        .await?;
    let rejected = client.expect_next().await?;
    // Then both classes name the offending field and neither reaches the effect boundary.
    assert_eq!(error_kind(&rejected), "invalid_params");
    assert_eq!(at(&rejected, "/error/data/field_path"), "params.op_id");
    assert_eq!(*fixture.gate.entered.borrow(), 0);
    assert!(!fixture.socket.path("effect").exists());
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}
