use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, params, request};

#[tokio::test]
async fn timed_out_work_keeps_its_connection_budget_and_control_remains_available() -> TestResult {
    // Given operations held on one connection, filling its advertised budget.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    let hello = client.hello("1", &fixture.socket.bearer).await?;
    let limit = usize::try_from(
        at(&hello, "/result/limits/max_in_flight_requests")
            .as_u64()
            .ok_or("limit")?,
    )?;
    let mut entered = fixture.gate.entered.subscribe();
    let operations: Vec<_> = (0..limit).map(|_| op_id()).collect();
    for (index, op) in operations.iter().enumerate() {
        client
            .send(&request(&format!("slow-{index}"), "doc.set", &params(op)))
            .await?;
        tokio::select! {
            result = entered.wait_for(|value| *value > index) => { result?; },
            reply = client.expect_next() => return Err(format!("premature settlement: {}", reply?).into()),
        }
    }
    tokio::time::timeout(
        Duration::from_secs(10),
        entered.wait_for(|value| *value == limit),
    )
    .await??;
    // When every RPC expires, the actual operations are still owned and bounded.
    let mut settled = HashSet::new();
    for _ in 0..limit {
        let reply = client.expect_next().await?;
        assert_eq!(error_kind(&reply), "deadline_exceeded");
        assert!(settled.insert(at(&reply, "/id").as_str().ok_or("id")?.to_owned()));
    }
    client
        .send(&request("overflow", "doc.set", &params(&op_id())))
        .await?;
    assert_eq!(error_kind(&client.expect_next().await?), "backpressure");
    // Then reserved operation control still works and there is no second reply for old IDs.
    let first = operations.first().ok_or("operation")?;
    client
        .send(&request("inspect", "op.get", &json!({"op_id":first})))
        .await?;
    assert_eq!(
        at(&client.expect_next().await?, "/result/state"),
        "accepted"
    );
    fixture.dispatch.shutdown().await?;
    client
        .send(&request("closed", "op.get", &json!({"op_id":first})))
        .await?;
    let closed = client.expect_next().await?;
    assert_eq!(at(&closed, "/id"), "closed");
    assert_eq!(error_kind(&closed), "unavailable");
    assert!(!fixture.socket.path("effect").exists());
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}
