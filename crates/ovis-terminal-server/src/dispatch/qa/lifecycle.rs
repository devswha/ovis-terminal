use std::time::Duration;

use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, params, request};

pub(crate) async fn deadline_keeps_work() -> TestResult {
    // Given a real socket and a mutation held before its commit boundary.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, std::sync::Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    let hello = client.hello("1", &fixture.socket.bearer).await?;
    assert_eq!(
        at(&hello, "/result/limits/rpc_default_deadline_ms"),
        "10000"
    );
    let op = op_id();
    let mut entered = fixture.gate.entered.subscribe();
    client
        .send(&request("slow", "doc.set", &params(&op)))
        .await?;
    tokio::time::timeout(
        Duration::from_secs(10),
        entered.wait_for(|value| *value > 0),
    )
    .await??;
    // When an unrelated operation-status query runs while the mutation is held.
    client
        .send(&request("status", "op.get", &json!({"op_id": op})))
        .await?;
    let status = client.expect_next().await?;
    assert_eq!(at(&status, "/id"), "status");
    assert_eq!(at(&status, "/result/state"), "accepted");
    // Then the deadline is an error with a handle, not a successful pending result.
    let deadline = client.expect_next().await?;
    assert_eq!(error_kind(&deadline), "deadline_exceeded");
    assert_eq!(at(&deadline, "/error/data/op_id"), &op);
    assert_eq!(at(&deadline, "/error/data/deadline_ms"), "10000");
    fixture.gate.release.notify_one();
    fixture.dispatch.drain().await?;
    client
        .send(&request("inspect", "op.get", &json!({"op_id": op})))
        .await?;
    let receipt = client.expect_next().await?;
    assert_eq!(at(&receipt, "/result/state"), "committed");
    assert_eq!(at(&receipt, "/result/result/value"), "payload");
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}
