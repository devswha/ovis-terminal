//! A connection keeps no authority after its credential is withdrawn.
//!
//! Driven over a real socket, dispatcher and operation ledger.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::credential_rotation_rig::{Rig, call, connected};
use crate::support::{TestResult, at, error_kind, op_id, request};

#[tokio::test]
async fn a_revoked_connection_is_refused_before_decoding_at_the_current_principal_epoch()
-> TestResult {
    // Given a writer and the owner connected through the revalidating handler.
    let rig = Rig::new()?;
    let writer = rig.writer()?;
    let server = rig.socket.serve(1, Arc::clone(&rig.handler)).await?;
    let mut owner = connected(&rig, &rig.socket.bearer).await?;
    let mut held = connected(&rig, writer.bearer()).await?;

    // When the owner withdraws the writer's credential.
    let revoked = call(
        &mut owner,
        "credential.revoke",
        &json!({"op_id": op_id(), "credential": writer.id().to_string()}),
    )
    .await?;

    // Then the result names the new epochs and the queued writes it rechecked once, and the
    // writer's open connection is refused for a malformed request before it is decoded.
    assert_eq!(
        at(&revoked, "/result/withdrawn/credential"),
        &json!(writer.id().to_string())
    );
    assert_eq!(at(&revoked, "/result/withdrawn/epoch"), "2");
    assert_eq!(at(&revoked, "/result/standing/principal"), "writer");
    assert_eq!(at(&revoked, "/result/standing/principal_epoch"), "2");
    assert_eq!(
        at(&revoked, "/result/writes/0/race"),
        &json!({"outcome": "cancelled_before_commit", "reason": "credential_revoked"})
    );
    assert_eq!(*rig.terminals.0.lock().map_err(|_| "poisoned")?, 1);
    let refused = call(
        &mut held,
        "doc.set",
        &json!({"op_id": op_id(), "name": "notes", "value": 1, "version": 0}),
    )
    .await?;
    assert_eq!(error_kind(&refused), "credential_revoked");
    assert_eq!(at(&refused, "/error/data/principal_epoch"), "2");
    let mut again = rig.socket.connect().await?;
    let relogin = again.hello("1", writer.bearer()).await?;
    assert_eq!(error_kind(&relogin), "credential_revoked");
    drop((owner, held, again));
    server.shutdown().await?;
    rig.close().await
}

#[tokio::test]
async fn rotation_hands_out_a_working_replacement_and_a_stale_epoch_conflicts() -> TestResult {
    // Given a writer whose principal epoch the owner read as 1.
    let rig = Rig::new()?;
    let writer = rig.writer()?;
    let server = rig.socket.serve(1, Arc::clone(&rig.handler)).await?;
    let mut owner = connected(&rig, &rig.socket.bearer).await?;
    let mut held = connected(&rig, writer.bearer()).await?;

    // When the writer tries to rotate itself, then the owner rotates it, then repeats the
    // rotation with the epoch it read before.
    let own = call(
        &mut held,
        "credential.rotate",
        &json!({"op_id": op_id(), "credential": writer.id().to_string(), "principal_epoch": "1"}),
    )
    .await?;
    let rotate_op = op_id();
    let rotated = call(
        &mut owner,
        "credential.rotate",
        &json!({"op_id": rotate_op, "credential": writer.id().to_string(), "principal_epoch": "1"}),
    )
    .await?;
    let bearer = at(&rotated, "/result/issued/bearer")
        .as_str()
        .ok_or("no replacement bearer")?
        .to_owned();
    let replacement = at(&rotated, "/result/issued/credential").clone();
    let stale = call(
        &mut owner,
        "credential.rotate",
        &json!({"op_id": op_id(), "credential": replacement, "principal_epoch": "1"}),
    )
    .await?;
    let retried = call(
        &mut owner,
        "credential.rotate",
        &json!({"op_id": rotate_op, "credential": writer.id().to_string(), "principal_epoch": "1"}),
    )
    .await?;

    // Then self-rotation is forbidden, the replacement works while the old connection is
    // refused, the stale epoch conflicts naming epoch 2, and a retry returns the same result
    // with the bearer shown once: the ledger keeps only its digest (IDEAL-108).
    assert_eq!(error_kind(&own), "forbidden");
    assert_eq!(at(&rotated, "/result/replaced/withdrawn/epoch"), "2");
    assert_eq!(
        at(&rotated, "/result/replaced/standing/principal_epoch"),
        "2"
    );
    assert_ne!(&replacement, &json!(writer.id().to_string()));
    assert_eq!(error_kind(&stale), "version_conflict");
    assert_eq!(at(&stale, "/error/data/principal_epoch"), "2");
    let mut sealed = at(&rotated, "/result").clone();
    let shown = ovis_terminal_core::telemetry::redaction::seal(&mut sealed, "/issued/bearer");
    assert_eq!(shown.as_deref(), Some(bearer.as_str()));
    assert_eq!(at(&retried, "/result"), &sealed);
    assert!(!retried.to_string().contains(&bearer));
    let mut fresh = connected(&rig, &bearer).await?;
    let inspect = call(&mut fresh, "op.get", &json!({"op_id": op_id()})).await?;
    assert_ne!(error_kind(&inspect), "credential_revoked");
    let old = call(&mut held, "op.get", &json!({"op_id": op_id()})).await?;
    assert_eq!(error_kind(&old), "credential_revoked");
    drop((owner, held, fresh));
    server.shutdown().await?;
    rig.close().await
}

#[tokio::test]
async fn a_write_parked_before_commit_is_refused_after_revocation_with_no_effect() -> TestResult {
    // Given a writer's document write admitted and parked before its commit.
    let rig = Rig::new()?;
    let writer = rig.writer()?;
    let server = rig.socket.serve(1, Arc::clone(&rig.handler)).await?;
    let mut owner = connected(&rig, &rig.socket.bearer).await?;
    let mut held = connected(&rig, writer.bearer()).await?;
    let mut entered = rig.parked.entered.subscribe();
    let write = op_id();
    held.send(&request(
        "parked",
        "doc.set",
        &json!({"op_id": write, "name": "notes", "value": 1, "version": "0"}),
    ))
    .await?;
    tokio::time::timeout(Duration::from_secs(10), entered.wait_for(|parked| *parked)).await??;

    // When the owner revokes the writer, then the barrier is released.
    let revoked = call(
        &mut owner,
        "credential.revoke",
        &json!({"op_id": op_id(), "credential": writer.id().to_string()}),
    )
    .await?;
    rig.parked.release.notify_one();

    // Then the commit-time recheck refuses the write at the new principal epoch and the
    // effect never ran.
    let refused = held.expect_next().await?;
    assert_eq!(at(&revoked, "/result/standing/principal_epoch"), "2");
    assert_eq!(error_kind(&refused), "credential_revoked");
    assert_eq!(at(&refused, "/error/data/principal_epoch"), "2");
    assert_eq!(at(&refused, "/error/data/op_id"), &json!(write));
    assert!(!*rig.parked.committed.lock().map_err(|_| "poisoned")?);
    drop((owner, held));
    server.shutdown().await?;
    rig.close().await
}
