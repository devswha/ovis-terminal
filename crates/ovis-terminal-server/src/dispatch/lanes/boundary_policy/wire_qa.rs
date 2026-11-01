//! The boundary policy in front of a real endpoint.
//!
//! Observers are denied with nothing reaching the handler, every call leaves one redacted
//! audit record, and error replies are redacted.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence, Permission,
    Presentation, PrincipalId, RemoteTransport, ResourceScope, ScopeResource, Session, UnixUid,
};
use ovis_terminal_core::telemetry::redaction::RetentionPolicy;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::dispatch::lanes::boundary_policy::{
    AuditTrail, BoundaryPolicy, MUTATION_ROLES,
};
use serde_json::{Value, json};

use super::support::socket::Fixture;
use super::support::{TestResult, at, error_kind, request, runtime};

const SENTINEL: &str = "hunter2-SENTINEL-58";

fn bearer_lookalike() -> String {
    format!("ovt1.c9.{}", "ab".repeat(32))
}

/// Counts what reaches it; `input.send` and `doc.set` fail with a bearer in their field path.
#[derive(Debug, Default)]
struct Counting(Arc<AtomicU64>);

impl RequestHandler for Counting {
    fn methods(&self) -> Vec<&'static str> {
        MUTATION_ROLES.iter().map(|(name, _)| *name).collect()
    }

    fn call(
        &self,
        call: Call,
    ) -> impl std::future::Future<Output = Result<Value, BoundaryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        let leaked = format!("params.{}", bearer_lookalike());
        let reply = match call.request.method() {
            "input.send" => Err(BoundaryError::domain(
                ErrorKind::from("forbidden".to_owned()),
                false,
            )
            .with_field_path(&leaked)),
            "doc.set" => Err(BoundaryError::invalid_params(Some(&leaked))),
            method => Ok(json!({"served": method})),
        };
        std::future::ready(reply)
    }
}

fn now() -> TestResult<TimestampMs> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

fn delegate(fixture: &Fixture, (principal, permission): (&str, Permission)) -> TestResult<String> {
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    let owner: Session = fixture
        .authority
        .authenticate(&admission, Presentation::new(&fixture.bearer, now()?))?;
    let scope = ResourceScope::new(runtime(1)?.machine_id(), ScopeResource::Machine);
    let request = IssueRequest::new(
        PrincipalId::parse(principal)?,
        vec![Grant::new(permission, scope)],
        TimestampMs::new(u64::MAX),
    );
    let issued = fixture
        .authority
        .issue(&owner, Issuance::new(request, now()?))?;
    Ok(issued.bearer().to_owned())
}

struct Served {
    fixture: Fixture,
    inner: Arc<AtomicU64>,
    trail: Arc<AuditTrail>,
    policy: Arc<BoundaryPolicy<Counting>>,
}

fn served() -> TestResult<Served> {
    let fixture = Fixture::new()?;
    let inner = Arc::new(AtomicU64::new(0));
    let policy = RetentionPolicy::new(std::num::NonZeroUsize::MIN.saturating_add(255), 60_000);
    let trail = Arc::new(AuditTrail::new(runtime(1)?, policy));
    let parts = (Arc::clone(&fixture.authority), Arc::clone(&trail));
    let policy = Arc::new(BoundaryPolicy::new(parts, Counting(Arc::clone(&inner))));
    Ok(Served {
        fixture,
        inner,
        trail,
        policy,
    })
}

#[tokio::test]
async fn an_observer_is_refused_every_mutation_before_the_handler_runs() -> TestResult {
    let served = served()?;
    let observer = delegate(&served.fixture, ("observer", Permission::Observe))?;
    let endpoint = served.fixture.serve(1, Arc::clone(&served.policy)).await?;
    let mut client = served.fixture.connect().await?;
    client.hello("1", &observer).await?;

    let mut refused = Vec::new();
    for (method, _) in MUTATION_ROLES
        .iter()
        .filter(|(name, _)| *name != "attach.open")
    {
        client.send(&request(method, method, &json!({}))).await?;
        let reply = client.expect_next().await?;
        refused.push((at(&reply, "/id").clone(), error_kind(&reply).to_owned()));
    }
    client
        .send(&request("attach", "attach.open", &json!({})))
        .await?;
    let attach = client.expect_next().await?;

    assert!(
        refused.iter().all(|(_, kind)| kind == "forbidden"),
        "{refused:?}"
    );
    assert_eq!(refused.len(), MUTATION_ROLES.len() - 1);
    assert_eq!(at(&attach, "/result/served"), "attach.open");
    assert_eq!(
        served.inner.load(Ordering::SeqCst),
        1,
        "only the attach reached it"
    );
    drop(client);
    endpoint.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn an_interactor_holds_no_takeover_prompt_raw_plugin_report_or_admin_authority() -> TestResult
{
    let served = served()?;
    let writer = delegate(&served.fixture, ("writer", Permission::Interact))?;
    let endpoint = served.fixture.serve(1, Arc::clone(&served.policy)).await?;
    let mut client = served.fixture.connect().await?;
    client.hello("1", &writer).await?;

    let guarded = [
        "attach.takeover",
        "prompt.answer",
        "input.raw",
        "plugin.invoke",
        "status.report",
        "machine.add",
        "credential.rotate",
        "update.stage",
    ];
    let mut kinds = Vec::new();
    for method in guarded {
        client.send(&request(method, method, &json!({}))).await?;
        kinds.push(error_kind(&client.expect_next().await?).to_owned());
    }
    client
        .send(&request("keep", "terminal.keep", &json!({})))
        .await?;
    let keep = client.expect_next().await?;

    assert_eq!(kinds, vec!["forbidden"; guarded.len()]);
    assert_eq!(at(&keep, "/result/served"), "terminal.keep");
    assert_eq!(
        served.inner.load(Ordering::SeqCst),
        1,
        "only the keep reached it"
    );
    drop(client);
    endpoint.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn audit_records_keep_ids_and_outcomes_and_no_secret_and_only_admins_read_them() -> TestResult
{
    let served = served()?;
    let writer = delegate(&served.fixture, ("writer", Permission::Interact))?;
    let endpoint = served.fixture.serve(1, Arc::clone(&served.policy)).await?;
    let mut client = served.fixture.connect().await?;
    client.hello("1", &writer).await?;

    let target = json!({"name": "build"});
    let params = json!({"op_id": "op-58", "target": target, "text": SENTINEL, "submit": "enter"});
    client.send(&request("send", "input.send", &params)).await?;
    let reply = client.expect_next().await?;
    let env = json!({"op_id": "op-59", "program": {"argv": ["/bin/echo", "a;b"]},
                     "env": {"API_KEY": SENTINEL}});
    client.send(&request("run", "terminal.run", &env)).await?;
    client.expect_next().await?;
    let document = json!({"op_id": "op-60", "name": "notes", "content": "x"});
    client.send(&request("doc", "doc.set", &document)).await?;
    let invalid = client.expect_next().await?;
    drop(client);
    endpoint.shutdown().await?;

    let owner = served.fixture.authority.authenticate(
        &EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?,
        Presentation::new(&served.fixture.bearer, now()?),
    )?;
    let trail = served
        .trail
        .read(&served.fixture.authority, (&owner, now()?))?;
    let denied = served.fixture.authority.authenticate(
        &EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?,
        Presentation::new(&writer, now()?),
    )?;
    let refused = served
        .trail
        .read(&served.fixture.authority, (&denied, now()?));
    let text = serde_json::to_string(&trail)?;

    assert_eq!(text.matches(SENTINEL).count(), 0, "{text}");
    assert_eq!(
        serde_json::to_string(&reply)?
            .matches(&bearer_lookalike())
            .count(),
        0
    );
    assert_eq!(at(&reply, "/error/data/kind"), "forbidden");
    assert_eq!(at(&reply, "/error/data/field_path"), "params.[REDACTED]");
    assert_eq!(at(&invalid, "/error/data/kind"), "invalid_params");
    assert_eq!(at(&invalid, "/error/data/field_path"), "params.[REDACTED]");
    assert_eq!(at(&trail, "/records/0/op_id"), "op-58");
    assert_eq!(at(&trail, "/records/0/seq"), "1");
    let machine = runtime(1)?.machine_id().to_string();
    assert_eq!(at(&trail, "/records/0/machine_id"), machine.as_str());
    let lifetime = runtime(1)?.runtime_id().to_string();
    assert_eq!(at(&trail, "/records/0/runtime_id"), lifetime.as_str());
    assert_eq!(at(&trail, "/records/0/target"), &target);
    assert_eq!(at(&trail, "/records/0/principal"), "writer");
    assert_eq!(at(&trail, "/records/0/error_kind"), "forbidden");
    assert_eq!(at(&trail, "/records/1/outcome"), "ok");
    assert_eq!(at(&trail, "/records/1/params/program/argv/1"), "a;b");
    assert_eq!(at(&trail, "/records/1/params/env/API_KEY"), "[REDACTED]");
    assert_eq!(
        refused
            .err()
            .map(|error| error.kind().to_owned())
            .as_deref(),
        Some("forbidden")
    );
    Ok(())
}
