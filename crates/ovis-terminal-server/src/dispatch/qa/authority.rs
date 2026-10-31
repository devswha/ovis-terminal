use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, IssuedCredential, LocalAccount, PeerEvidence,
    Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope, Revocation,
    ScopeResource, Session, UnixUid,
};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, params, request, runtime};

fn now() -> TestResult<TimestampMs> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

fn owner(fixture: &Fixture) -> TestResult<Session> {
    let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)));
    let admission = policy.admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    Ok(fixture.socket.authority.authenticate(
        &admission,
        Presentation::new(&fixture.socket.bearer, now()?),
    )?)
}

fn issue(fixture: &Fixture, principal: &str) -> TestResult<IssuedCredential<RegistryIdentities>> {
    let scope = ResourceScope::new(
        runtime(1)?.machine_id(),
        ScopeResource::Document {
            name: DocName::new("test".to_owned()),
        },
    );
    let grants = [Permission::Observe, Permission::Interact]
        .into_iter()
        .map(|permission| Grant::new(permission, scope.clone()))
        .collect();
    let request = IssueRequest::new(
        PrincipalId::parse(principal)?,
        grants,
        TimestampMs::new(u64::MAX),
    );
    Ok(fixture
        .socket
        .authority
        .issue(&owner(fixture)?, Issuance::new(request, now()?))?)
}

#[tokio::test]
async fn resource_scope_is_checked_before_handlers_and_receipts_are_principal_bound() -> TestResult
{
    // Given a credential with access to only document "test".
    let fixture = Fixture::new()?;
    let issued = issue(&fixture, "limited")?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", issued.bearer()).await?;
    // When it tries another document, then an authorized operation.
    let mut denied = params(&op_id());
    *denied.get_mut("name").ok_or("name")? = json!("other");
    client.send(&request("denied", "doc.set", &denied)).await?;
    assert_eq!(error_kind(&client.expect_next().await?), "forbidden");
    assert_eq!(*fixture.gate.entered.borrow(), 0);
    let op = op_id();
    fixture.gate.release.notify_one();
    client
        .send(&request("allowed", "doc.set", &params(&op)))
        .await?;
    assert_eq!(at(&client.expect_next().await?, "/result/value"), "payload");
    // Then the original narrowed principal can inspect/cancel, but another principal cannot.
    client
        .send(&request("inspect", "op.get", &json!({"op_id":op})))
        .await?;
    assert_eq!(
        at(&client.expect_next().await?, "/result/state"),
        "committed"
    );
    client
        .send(&request("cancel", "op.cancel", &json!({"op_id":op})))
        .await?;
    assert_eq!(
        at(&client.expect_next().await?, "/result/outcome"),
        "already_committed"
    );
    let mut other = fixture.socket.connect().await?;
    other.hello("1", &fixture.socket.bearer).await?;
    other
        .send(&request("foreign", "op.get", &json!({"op_id":op})))
        .await?;
    assert_eq!(error_kind(&other.expect_next().await?), "not_found");
    drop((client, other));
    server.shutdown().await?;
    fixture.close().await
}

#[tokio::test]
async fn revocation_after_admission_is_rechecked_at_the_effect_boundary() -> TestResult {
    // Given an authorized operation held before its effect.
    let fixture = Fixture::new()?;
    let issued = issue(&fixture, "limited")?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", issued.bearer()).await?;
    let op = op_id();
    let mut entered = fixture.gate.entered.subscribe();
    client
        .send(&request("queued", "doc.set", &params(&op)))
        .await?;
    tokio::time::timeout(
        Duration::from_secs(10),
        entered.wait_for(|count| *count > 0),
    )
    .await??;
    // When its credential is revoked after dispatch but before the commit check.
    fixture
        .socket
        .authority
        .revoke(&owner(&fixture)?, &Revocation::new(*issued.id(), now()?))?;
    fixture.gate.release.notify_one();
    // Then no effect occurs, and a fresh credential of the same principal sees the failure.
    assert_eq!(
        error_kind(&client.expect_next().await?),
        "credential_revoked"
    );
    assert!(!fixture.socket.path("effect").exists());
    let fresh = issue(&fixture, "limited")?;
    let mut inspector = fixture.socket.connect().await?;
    inspector.hello("1", fresh.bearer()).await?;
    inspector
        .send(&request("inspect", "op.get", &json!({"op_id":op})))
        .await?;
    let retained = inspector.expect_next().await?;
    assert_eq!(at(&retained, "/result/state"), "failed");
    assert_eq!(at(&retained, "/result/error/kind"), "credential_revoked");
    drop((client, inspector));
    server.shutdown().await?;
    fixture.close().await
}
