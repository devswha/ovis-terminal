//! Fair FIFO admission under the pinned-snapshot budget, over the real socket.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};

use super::admission_fixture::{Fixture, TestResult, at, error_kind, op_id, request, set};

const WRITERS: usize = 32;

fn decimal(value: &Value) -> TestResult<u64> {
    Ok(value.as_str().ok_or("not a decimal string")?.parse()?)
}

fn one_through(last: u64) -> BTreeSet<u64> {
    (1..=last).collect()
}

/// 32 writers on one document, four times the pinned-snapshot budget, all commit.
pub(crate) async fn happy() -> TestResult {
    // Given 32 writers, each on its own authenticated connection to one document.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut clients = Vec::new();
    for _ in 0..WRITERS {
        clients.push(fixture.client().await?);
    }

    // When every writer submits before any reply is read.
    for (index, (client, _)) in clients.iter_mut().enumerate() {
        let id = format!("writer-{index}");
        client.send(&set(&id, &op_id(), &id)).await?;
    }
    let mut replies = Vec::new();
    for (client, _) in &mut clients {
        replies.push(client.expect_next().await?);
    }

    // Then every writer commits once, in one serial order, with no lost update.
    let refused: Vec<&Value> = replies
        .iter()
        .filter(|reply| reply.get("error").is_some())
        .collect();
    assert_eq!(refused, Vec::<&Value>::new(), "no writer may be refused");
    let mut versions = BTreeSet::new();
    for reply in &replies {
        versions.insert(decimal(at(reply, "/result/version"))?);
    }
    assert_eq!(versions, one_through(32));
    let last = replies
        .iter()
        .find(|reply| at(reply, "/result/version") == "32")
        .ok_or("final version")?;
    assert_eq!(at(last, "/result/writers"), "32");
    // And the budget is still exactly eight slots, all returned.
    let (_, hello) = clients.first().ok_or("client")?;
    assert_eq!(at(hello, "/result/limits/queued_operations"), "32");
    let leases = fixture.pin_all()?;
    assert_eq!(leases.len(), 8);
    fixture.release(leases)?;
    drop(clients);
    server.shutdown().await?;
    fixture.close().await
}

/// Bound + 1 is refused naming the bound; a queued deadline reports the position.
pub(crate) async fn failure() -> TestResult {
    // Given every pinned-snapshot slot held and one connection.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let (mut client, hello) = fixture.client().await?;
    let bound = decimal(at(&hello, "/result/limits/queued_operations"))?;
    let leases = fixture.pin_all()?;
    assert_eq!(leases.len(), 8);

    // When the connection submits one more operation than its queue bound.
    let ops: Vec<String> = (0..=bound).map(|_| op_id()).collect();
    for (index, op) in ops.iter().enumerate() {
        client
            .send(&set(&format!("q-{index}"), op, &format!("queued-{index}")))
            .await?;
    }

    // Then exactly one is refused at once, naming the bound it reached.
    let refused = client.expect_next().await?;
    assert_eq!(error_kind(&refused), "limit_exceeded");
    assert_eq!(at(&refused, "/error/data/bound/limit"), "queued_operations");
    assert_eq!(decimal(at(&refused, "/error/data/bound/value"))?, bound);
    assert_eq!(at(&refused, "/error/data/retryable"), true);
    let refused_index: usize = at(&refused, "/id")
        .as_str()
        .and_then(|id| id.strip_prefix("q-"))
        .ok_or("refused id")?
        .parse()?;
    let refused_op = ops.get(refused_index).ok_or("refused op")?.clone();
    let queued: Vec<&String> = ops.iter().filter(|op| **op != refused_op).collect();

    // And every other operation is inspectable as queued at a distinct FIFO place.
    for op in &queued {
        client
            .send(&request(
                &format!("get-{op}"),
                "op.get",
                &json!({"op_id": op}),
            ))
            .await?;
    }
    let mut positions = BTreeSet::new();
    for _ in &queued {
        let status = client.expect_next().await?;
        assert_eq!(at(&status, "/result/state"), "queued", "{status}");
        assert_eq!(at(&status, "/result/method"), "doc.set");
        positions.insert(decimal(at(&status, "/result/queued_position"))?);
    }
    assert_eq!(positions, one_through(bound));

    // When the request deadline passes while they are still queued.
    let mut expired = BTreeSet::new();
    for _ in &queued {
        let reply = client.expect_next().await?;
        assert_eq!(error_kind(&reply), "deadline_exceeded", "{reply}");
        assert!(
            queued
                .iter()
                .any(|op| at(&reply, "/error/data/op_id") == op.as_str())
        );
        expired.insert(decimal(at(&reply, "/error/data/queued_position"))?);
    }

    // Then each deadline carried its place, and releasing the slots admits all of them.
    assert_eq!(expired, one_through(bound));
    fixture.release(leases)?;
    fixture.dispatch.drain().await?;
    let mut versions = BTreeSet::new();
    for op in &queued {
        client
            .send(&request("inspect", "op.get", &json!({"op_id": op})))
            .await?;
        let receipt = client.expect_next().await?;
        assert_eq!(at(&receipt, "/result/state"), "committed", "{receipt}");
        assert_eq!(at(&receipt, "/result/queued_position"), &Value::Null);
        versions.insert(decimal(at(&receipt, "/result/result/version"))?);
    }
    assert_eq!(versions, one_through(bound));
    // And the refused operation was never admitted: no record, no effect.
    client
        .send(&request("refused", "op.get", &json!({"op_id": refused_op})))
        .await?;
    assert_eq!(error_kind(&client.expect_next().await?), "not_found");
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}
