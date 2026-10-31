//! Concurrent operation reuse through the actual persistent socket.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::support::{Fixture, TestResult, at, op_id, params, request};

#[tokio::test]
async fn eight_identical_dispatches_converge_on_one_execution_and_outcome() -> TestResult {
    // Given eight correlated calls sharing one operation and a held effect.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    let op = op_id();
    let mut entered = fixture.gate.entered.subscribe();
    for index in 0..8 {
        client
            .send(&request(
                &format!("duplicate-{index}"),
                "doc.set",
                &params(&op),
            ))
            .await?;
    }
    tokio::time::timeout(Duration::from_secs(5), entered.wait_for(|count| *count > 0)).await??;
    client
        .send(&request("accepted", "op.get", &json!({"op_id":op})))
        .await?;
    let accepted = client.expect_next().await?;
    assert_eq!(at(&accepted, "/id"), "accepted");
    assert_eq!(at(&accepted, "/result/state"), "accepted");
    // When the single operation may complete, all outstanding callers must converge.
    fixture.gate.release.notify_waiters();
    let mut replies = Vec::new();
    for _ in 0..8 {
        replies.push(client.expect_next().await?);
    }
    client
        .send(&request("retained", "op.get", &json!({"op_id":op})))
        .await?;
    let retained = client.expect_next().await?;
    let effects = std::fs::read(fixture.socket.path("effect"))?;
    let preparations = *fixture.gate.entered.borrow();
    drop(client);
    server.shutdown().await?;
    fixture.close().await?;
    // Then each RPC has its own reply, but all carry the one retained result and op_id.
    let ids: HashSet<_> = replies
        .iter()
        .map(|reply| at(reply, "/id").clone())
        .collect();
    assert_eq!(ids.len(), 8);
    for reply in &replies {
        assert!(
            reply.get("error").is_none(),
            "duplicate did not converge: {reply}"
        );
        assert_eq!(at(reply, "/result"), at(&retained, "/result/result"));
        assert_eq!(at(reply, "/result/op_id"), &op);
    }
    assert_eq!(at(&retained, "/result/state"), "committed");
    assert_eq!(at(&retained, "/result/op_id"), &op);
    assert_eq!(preparations, 1);
    assert_eq!(effects, b"\"payload\"");
    Ok(())
}
