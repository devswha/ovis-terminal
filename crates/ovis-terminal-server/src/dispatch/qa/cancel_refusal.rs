//! Cancellations that must not be delivered: unknown, foreign and already completed.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence, Permission,
    Presentation, PrincipalId, RemoteTransport, ResourceScope, ScopeResource, UnixUid,
};
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::json;

use super::cancel_fixture::{Fixture, TestResult, at, error_kind, op_id, request, runtime, set};

fn now() -> TestResult<TimestampMs> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

/// A bearer for another principal, scoped to the document it may write.
fn foreign(fixture: &Fixture) -> TestResult<String> {
    let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)));
    let admission = policy.admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    let authority = &fixture.socket.authority;
    let owner = authority.authenticate(
        &admission,
        Presentation::new(&fixture.socket.bearer, now()?),
    )?;
    let scope = ResourceScope::new(
        runtime(1)?.machine_id(),
        ScopeResource::Document {
            name: DocName::new("other".to_owned()),
        },
    );
    let grants = [Permission::Observe, Permission::Interact]
        .into_iter()
        .map(|permission| Grant::new(permission, scope.clone()))
        .collect();
    let request = IssueRequest::new(
        PrincipalId::parse("foreign")?,
        grants,
        TimestampMs::new(u64::MAX),
    );
    let issued = authority.issue(&owner, Issuance::new(request, now()?))?;
    Ok(issued.bearer().to_owned())
}

pub(crate) async fn failure() -> TestResult {
    // Given an owner connection, a foreign principal and a committed operation.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut owner = fixture.client(&fixture.socket.bearer).await?;
    let bearer = foreign(&fixture)?;
    let mut other = fixture.client(&bearer).await?;
    let done = op_id();
    fixture.gate.release.notify_one();
    owner.send(&set("done", &done, ("done", "kept"))).await?;
    let receipt = owner.expect_next().await?;
    assert_eq!(at(&receipt, "/result/value"), "kept", "{receipt}");

    // When the identity is unknown, already completed, or another principal's.
    owner
        .send(&request("unknown", "op.cancel", &json!({"op_id": op_id()})))
        .await?;
    assert_eq!(error_kind(&owner.expect_next().await?), "not_found");
    owner
        .send(&request("late", "op.cancel", &json!({"op_id": done})))
        .await?;
    let late = owner.expect_next().await?;
    let held = op_id();
    owner.send(&set("held", &held, ("held", "v"))).await?;
    owner.send(&set("probe", &held, ("held", "other"))).await?;
    // One connection dispatches both concurrently: either may reserve the identity, and
    // the other's op_reused reply proves the winner is reserved.
    let loser = owner.expect_next().await?;
    assert_eq!(error_kind(&loser), "op_reused", "{loser}");
    let (winner, value) = if at(&loser, "/id") == "held" {
        ("probe", "other")
    } else {
        ("held", "v")
    };
    for op in [&done, &held] {
        other
            .send(&request("foreign", "op.cancel", &json!({"op_id": op})))
            .await?;
        assert_eq!(error_kind(&other.expect_next().await?), "forbidden");
    }

    // Then nothing was cancelled: the receipt is retained and the held work still commits.
    assert_eq!(at(&late, "/result/outcome"), "already_committed", "{late}");
    assert_eq!(at(&late, "/result/state"), "committed");
    assert_eq!(at(&late, "/result/result"), at(&receipt, "/result"));
    fixture.gate.release.notify_one();
    let committed = owner.expect_next().await?;
    assert_eq!(at(&committed, "/id"), winner, "{committed}");
    assert_eq!(at(&committed, "/result/value"), value, "{committed}");
    assert_eq!(
        std::fs::read(fixture.effect("held"))?,
        format!("\"{value}\"").into_bytes()
    );
    drop((owner, other));
    server.shutdown().await?;
    fixture.close().await
}
