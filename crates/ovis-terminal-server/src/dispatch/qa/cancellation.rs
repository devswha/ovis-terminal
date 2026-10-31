use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, params, request};

struct ReleaseEffect(Arc<super::support::Gate>);

impl Drop for ReleaseEffect {
    fn drop(&mut self) {
        *self
            .0
            .effect_released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.0.effect_signal.notify_all();
    }
}

#[tokio::test]
async fn cancellation_before_commit_prevents_the_effect() -> TestResult {
    // Given accepted work held before its single-use effect boundary.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
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
    // When cancellation commits before permission to start is granted.
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id":op})))
        .await?;
    let first = client.expect_next().await?;
    let second = client.expect_next().await?;
    let (cancel, slow) = if at(&first, "/id") == "cancel" {
        (first, second)
    } else {
        (second, first)
    };
    // Then cancellation and the original request settle honestly, with no file effect.
    assert_eq!(at(&cancel, "/result/outcome"), "cancelled_before_commit");
    assert_eq!(error_kind(&slow), "cancelled");
    assert!(!fixture.socket.path("effect").exists());
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}

#[tokio::test]
async fn disconnect_does_not_abandon_an_accepted_operation() -> TestResult {
    // Given an accepted request and a second authenticated connection for inspection.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    let op = op_id();
    let mut entered = fixture.gate.entered.subscribe();
    client
        .send(&request("lost", "doc.set", &params(&op)))
        .await?;
    tokio::time::timeout(
        Duration::from_secs(10),
        entered.wait_for(|value| *value > 0),
    )
    .await??;
    // When the requester disappears before the real effect is released.
    drop(client);
    fixture.gate.release.notify_one();
    fixture.dispatch.drain().await?;
    let mut reader = fixture.socket.connect().await?;
    reader.hello("1", &fixture.socket.bearer).await?;
    reader
        .send(&request("inspect", "op.get", &json!({"op_id":op})))
        .await?;
    // Then its durable result and actual effect remain inspectable.
    assert_eq!(
        at(&reader.expect_next().await?, "/result/state"),
        "committed"
    );
    assert_eq!(
        std::fs::read(fixture.socket.path("effect"))?,
        b"\"payload\""
    );
    drop(reader);
    server.shutdown().await?;
    fixture.close().await
}

#[tokio::test]
async fn shutdown_cancels_precommit_work_and_joins_it() -> TestResult {
    // Given work waiting at a cooperative precommit barrier.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    let mut entered = fixture.gate.entered.subscribe();
    client
        .send(&request("held", "doc.set", &params(&op_id())))
        .await?;
    tokio::time::timeout(
        Duration::from_secs(10),
        entered.wait_for(|value| *value > 0),
    )
    .await??;

    // When dispatch shuts down, without manually releasing the handler.
    tokio::time::timeout(Duration::from_secs(10), fixture.dispatch.shutdown()).await??;
    // Then the waiter was cancelled before effects and the task has been joined.
    assert_eq!(error_kind(&client.expect_next().await?), "cancelled");
    assert!(!fixture.socket.path("effect").exists());
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}

#[tokio::test]
async fn cancellation_after_effect_start_is_pending_without_rollback() -> TestResult {
    // Given an operation held inside its real effect after running intent commits.
    let fixture = Fixture::new()?;
    *fixture
        .gate
        .effect_released
        .lock()
        .map_err(|_| "gate poisoned")? = false;
    let release = ReleaseEffect(Arc::clone(&fixture.gate));
    let mut started = fixture.gate.effect_started.subscribe();
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    let op = op_id();
    fixture.gate.release.notify_one();
    client
        .send(&request("effect", "doc.set", &params(&op)))
        .await?;
    tokio::time::timeout(Duration::from_secs(10), started.wait_for(|value| *value)).await??;
    // When cancellation arrives after permission to perform the effect.
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id":op})))
        .await?;
    let cancel = client.expect_next().await?;
    // Release before assertions, so an assertion failure cannot strand the blocking worker.
    drop(release);
    assert_eq!(at(&cancel, "/result/outcome"), "pending");
    assert_eq!(at(&cancel, "/result/evidence/cancellation_requested"), true);
    let result = client.expect_next().await?;
    assert_eq!(at(&result, "/id"), "effect");
    // Then completion is truthful and the file was never rolled back.
    assert_eq!(
        std::fs::read(fixture.socket.path("effect"))?,
        b"\"payload\""
    );
    client
        .send(&request("inspect", "op.get", &json!({"op_id":op})))
        .await?;
    assert_eq!(
        at(&client.expect_next().await?, "/result/state"),
        "committed"
    );
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}

#[tokio::test]
async fn shutdown_join_does_not_hold_the_request_admission_lock() -> TestResult {
    // Given an effect which must be joined, not aborted, during shutdown.
    let fixture = Fixture::new()?;
    *fixture
        .gate
        .effect_released
        .lock()
        .map_err(|_| "gate poisoned")? = false;
    let release = ReleaseEffect(Arc::clone(&fixture.gate));
    let mut started = fixture.gate.effect_started.subscribe();
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    let op = op_id();
    fixture.gate.release.notify_one();
    client
        .send(&request("running", "doc.set", &params(&op)))
        .await?;
    tokio::time::timeout(Duration::from_secs(10), started.wait_for(|value| *value)).await??;
    // When shutdown is polled before a new request, the new request must not wait on the join.
    let (shutdown, reply) = tokio::join!(
        biased;
        fixture.dispatch.shutdown(),
        async {
            client.send(&request("during-shutdown", "op.get", &json!({"op_id":op}))).await?;
            let reply = client.expect_next().await;
            drop(release);
            reply
        }
    );
    shutdown?;
    let reply = reply?;
    // Then admission refuses promptly while shutdown still joins the real effect.
    assert_eq!(at(&reply, "/id"), "during-shutdown");
    assert_eq!(error_kind(&reply), "unavailable");
    assert_eq!(at(&client.expect_next().await?, "/id"), "running");
    assert_eq!(
        std::fs::read(fixture.socket.path("effect"))?,
        b"\"payload\""
    );
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}
