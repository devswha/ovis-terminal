//! doc.get is an unrecorded read, but it is still authorized: only Observe on the document
//! itself may read it.

use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence, Permission,
    Presentation, PrincipalId, RemoteTransport, ResourceScope, ScopeResource, UnixUid,
};
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::json;

use super::socket::{at, error_kind, request, runtime};
use super::{Served, TestResult, call, op, set};

fn now() -> TestResult<TimestampMs> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

/// A bearer for `principal` holding `permission` on document `name` only.
fn credential(
    served: &Served,
    principal: &str,
    (permission, name): (Permission, &str),
) -> TestResult<String> {
    let authority = &served.socket.authority;
    let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)));
    let admission = policy.admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    let owner =
        authority.authenticate(&admission, Presentation::new(&served.socket.bearer, now()?))?;
    let scope = ResourceScope::new(
        runtime(1)?.machine_id(),
        ScopeResource::Document {
            name: DocName::new(name.to_owned()),
        },
    );
    let request = IssueRequest::new(
        PrincipalId::parse(principal)?,
        vec![Grant::new(permission, scope)],
        TimestampMs::new(u64::MAX),
    );
    Ok(authority
        .issue(&owner, Issuance::new(request, now()?))?
        .bearer()
        .to_owned())
}

#[tokio::test]
async fn doc_get_requires_observe_on_that_document() -> TestResult {
    // Given: "shared" committed by the owner, and three narrowed principals.
    let served = Served::start().await?;
    let mut owner = served.client().await?;
    call(
        &mut owner,
        &set(
            &op(0)?,
            json!({"name": "shared", "version": "0", "value": {"x": 1}}),
        ),
    )
    .await?;
    let principals = [
        (
            "elsewhere",
            (Permission::Observe, "other"),
            Some("forbidden"),
        ),
        (
            "typist",
            (Permission::Interact, "shared"),
            Some("forbidden"),
        ),
        ("reader", (Permission::Observe, "shared"), None),
    ];

    for (principal, grant, refused) in principals {
        // When: each reads "shared" on its own connection.
        let bearer = credential(&served, principal, grant)?;
        let mut client = served.socket.connect().await?;
        client.hello("1", &bearer).await?;
        let reply = call(
            &mut client,
            &request("get", "doc.get", &json!({"name": "shared"})),
        )
        .await?;

        // Then: only Observe on "shared" itself reads it; the others are typed forbidden.
        match refused {
            Some(kind) => {
                assert_eq!(error_kind(&reply), kind, "{principal}: {reply}");
                assert_eq!(
                    at(&reply, "/result"),
                    &serde_json::Value::Null,
                    "{principal}"
                );
            }
            None => assert_eq!(
                at(&reply, "/result"),
                &json!({"name": "shared", "version": "1", "value": {"x": 1}}),
                "{principal}: {reply}"
            ),
        }
    }
    drop(owner);
    served.close().await
}
