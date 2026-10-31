//! Removal needs the admin grant; a principal that may change layout still cannot remove.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence, Permission,
    Presentation, PrincipalId, RemoteTransport, ResourceScope, ScopeResource, UnixUid,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::json;

use super::support::{TestResult, at, error_kind, request, runtime};
use super::worktree_fixture::Fixture;

fn now() -> TestResult<TimestampMs> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(TimestampMs::new(u64::try_from(elapsed.as_millis())?))
}

/// Issues an observe+interact credential over the whole machine to `principal`.
fn interactor(fixture: &Fixture, principal: &str) -> TestResult<String> {
    let authority = &fixture.socket.authority;
    let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)));
    let admission = policy.admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    let owner = authority.authenticate(
        &admission,
        Presentation::new(&fixture.socket.bearer, now()?),
    )?;
    let scope = ResourceScope::new(runtime(1)?.machine_id(), ScopeResource::Machine);
    let grants = [Permission::Observe, Permission::Interact]
        .into_iter()
        .map(|permission| Grant::new(permission, scope.clone()))
        .collect();
    let request = IssueRequest::new(
        PrincipalId::parse(principal)?,
        grants,
        TimestampMs::new(u64::MAX),
    );
    let issued = authority.issue(&owner, Issuance::new(request, now()?))?;
    Ok(issued.bearer().to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn removal_needs_the_admin_grant_beyond_layout_change() -> TestResult {
    // Given a principal holding observe and interact over the whole machine.
    let fixture = Fixture::new(PathBuf::from("git"))?;
    let bearer = interactor(&fixture, "agent")?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &bearer).await?;
    // When it creates a checkout, which it may, and then asks to remove it with full scope.
    let list = json!({"repository_id": "repo:main"});
    client
        .send(&request("list", "worktree.list", &list))
        .await?;
    let listed = client.expect_next().await?;
    let source = at(&listed, "/result/worktrees/0/checkout_id").clone();
    let create = json!({"op_id": uuid::Uuid::now_v7().to_string(), "repository_id": "repo:main",
        "source": source, "branch": "agent-made"});
    client
        .send(&request("create", "worktree.create", &create))
        .await?;
    let created = client.expect_next().await?;
    let checkout = at(&created, "/result/worktree/checkout_id").clone();
    let path = at(&created, "/result/worktree/path")
        .as_str()
        .ok_or("path")?
        .to_owned();
    let remove = json!({"op_id": uuid::Uuid::now_v7().to_string(), "checkout_id": checkout,
        "scope": {"group": true, "force": true}});
    client
        .send(&request("remove", "worktree.remove", &remove))
        .await?;
    let refused = client.expect_next().await?;
    // Then removal is forbidden before any effect and the checkout stays.
    assert_eq!(error_kind(&refused), "forbidden", "{refused}");
    assert!(Path::new(&path).is_dir());
    client
        .send(&request("after", "worktree.list", &list))
        .await?;
    let after = client.expect_next().await?;
    assert_eq!(at(&after, "/result/worktrees/1/checkout_id"), &checkout);
    drop(client);
    fixture.close(server).await
}
