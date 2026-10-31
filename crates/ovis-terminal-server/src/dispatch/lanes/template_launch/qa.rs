//! The template launch lane over a real dispatcher, store, authority and socket.
//!
//! The launcher records reservations and writes in memory; readiness is fed to the real
//! wait hub of each reservation, so gating, the write-boundary rechecks and the single
//! release run unchanged.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{
    EndpointPolicy, Grant, Issuance, IssueRequest, IssuedCredential, LocalAccount, PeerEvidence,
    Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope, Revocation,
    ScopeResource, Session, UnixUid,
};
use ovis_terminal_core::leases::{Acquire, Caller, ControlSlot};
use ovis_terminal_core::registry::{LeaseId, RegistryIdentities};
use ovis_terminal_core::waits::Waits;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::dispatch::lanes::template_launch::{
    GetFirstMessage, RunTerminal, TemplateLaunchLane,
};
use ovis_terminal_services::templates::TemplateCatalog;
use serde_json::{Value, json};

use super::support::{Fixture, TestResult, at, request, runtime};
use super::template_launch_memory::{Memory, MemoryLauncher, Ready, idle};

fn now() -> TestResult<TimestampMs> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

fn owner(fixture: &Fixture) -> TestResult<Session> {
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    Ok(fixture.socket.authority.authenticate(
        &admission,
        Presentation::new(&fixture.socket.bearer, now()?),
    )?)
}

struct Lane {
    fixture: Fixture,
    lane: Arc<TemplateLaunchLane<MemoryLauncher>>,
}

/// A dispatcher serving `terminal.run` and `terminal.first_message` over one template.
fn lane(ready: Ready) -> TestResult<Lane> {
    let mut fixture = Fixture::new()?;
    let dir = fixture.socket.path("templates");
    std::fs::create_dir(&dir)?;
    let template =
        json!({"name": "build", "version": "1", "description": "builder", "argv": ["/bin/cat"]});
    std::fs::write(dir.join("build.json"), template.to_string())?;
    let launcher = MemoryLauncher {
        state: Mutex::new(Memory {
            ready,
            hubs: Vec::new(),
            writes: Vec::new(),
        }),
        authority: Arc::clone(&fixture.socket.authority),
    };
    let authority = Arc::clone(&fixture.socket.authority);
    let lane = TemplateLaunchLane::new(
        (TemplateCatalog::new(dir), launcher),
        authority,
        runtime(1)?.machine_id(),
    );
    let dispatch = Arc::get_mut(&mut fixture.dispatch).ok_or("dispatcher shared")?;
    dispatch.register(RunTerminal::new(&lane))?;
    dispatch.register(GetFirstMessage::new(&lane))?;
    Ok(Lane { fixture, lane })
}

fn launch(op: &str) -> Value {
    json!({"op_id": op, "template": {"name": "build", "version": "1"},
        "geometry": {"cols": 80, "rows": 24}, "first_message": {"text": "go", "deadline_ms": "10000"}})
}

fn writes(lane: &Lane) -> TestResult<Vec<Vec<u8>>> {
    Ok(lane
        .lane
        .launcher()
        .state
        .lock()
        .map_err(|_| "poisoned")?
        .writes
        .clone())
}

fn hub(lane: &Lane) -> TestResult<Waits> {
    let state = lane.lane.launcher().state.lock().map_err(|_| "poisoned")?;
    Ok(state.hubs.first().ok_or("no reservation")?.0.clone())
}

async fn settled(client: &mut super::support::socket::Client, op: &str) -> TestResult<Value> {
    let params = json!({"launch_op": op, "settled_within_ms": "10000"});
    client
        .send(&request("settled", "terminal.first_message", &params))
        .await?;
    client.expect_next().await
}

fn op() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_declared_while_starting_is_not_lost_and_releases_once() -> TestResult {
    let lane = lane(Ready::AtStart)?;
    let server = lane
        .fixture
        .socket
        .serve(1, Arc::clone(&lane.fixture.dispatch))
        .await?;
    let mut client = lane.fixture.socket.connect().await?;
    client.hello("1", &lane.fixture.socket.bearer).await?;
    let launch_op = op();
    client
        .send(&request("launch", "terminal.run", &launch(&launch_op)))
        .await?;
    let launched = client.expect_next().await?;
    assert_eq!(
        at(&launched, "/result/first_message/launch_op"),
        &json!(launch_op),
        "{launched}"
    );
    let sent = settled(&mut client, &launch_op).await?;
    assert_eq!(
        (at(&sent, "/result/state"), at(&sent, "/result/queued_seq")),
        (&json!("sent"), &json!("1")),
        "{sent}"
    );
    assert_eq!(writes(&lane)?, vec![b"go\r".to_vec()]);
    server.shutdown().await?;
    drop(lane.lane);
    lane.fixture.close().await
}

fn issue(lane: &Lane, owner: &Session) -> TestResult<IssuedCredential<RegistryIdentities>> {
    let scope = ResourceScope::new(runtime(1)?.machine_id(), ScopeResource::Machine);
    let grants = [Permission::Observe, Permission::Interact]
        .into_iter()
        .map(|permission| Grant::new(permission, scope.clone()))
        .collect();
    let request = IssueRequest::new(
        PrincipalId::parse("launcher")?,
        grants,
        TimestampMs::new(u64::MAX),
    );
    Ok(lane
        .fixture
        .socket
        .authority
        .issue(owner, Issuance::new(request, now()?))?)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_requester_or_a_held_lease_cancels_the_message_with_zero_writes() -> TestResult {
    for revoke in [true, false] {
        let lane = lane(Ready::Later)?;
        let owner = owner(&lane.fixture)?;
        let issued = issue(&lane, &owner)?;
        let server = lane
            .fixture
            .socket
            .serve(1, Arc::clone(&lane.fixture.dispatch))
            .await?;
        let mut client = lane.fixture.socket.connect().await?;
        client.hello("1", issued.bearer()).await?;
        let launch_op = op();
        client
            .send(&request("launch", "terminal.run", &launch(&launch_op)))
            .await?;
        let launched = client.expect_next().await?;
        assert_eq!(
            at(&launched, "/result/first_message/state"),
            "accepted",
            "{launched}"
        );
        if revoke {
            lane.fixture
                .socket
                .authority
                .revoke(&owner, &Revocation::new(*issued.id(), now()?))?;
            let fresh = issue(&lane, &owner)?;
            client = lane.fixture.socket.connect().await?;
            client.hello("1", fresh.bearer()).await?;
        } else {
            let mut state = lane.lane.launcher().state.lock().map_err(|_| "poisoned")?;
            let (_, leases) = state.hubs.first_mut().ok_or("no reservation")?;
            let tui = ClientId::new("tui".to_owned());
            leases.acquire(
                Caller::new(&owner, &tui, now()?),
                Acquire::new(ControlSlot::Interact, LeaseId::try_from(1)?),
            )?;
            drop(state);
        }
        idle(&hub(&lane)?)?;
        let record = settled(&mut client, &launch_op).await?;
        let reason = if revoke {
            "authority_lapsed"
        } else {
            "lease_held"
        };
        assert_eq!(
            (at(&record, "/result/state"), at(&record, "/result/reason")),
            (&json!("cancelled"), &json!(reason)),
            "{record}"
        );
        assert_eq!(writes(&lane)?, Vec::<Vec<u8>>::new());
        server.shutdown().await?;
        drop(lane.lane);
        lane.fixture.close().await?;
    }
    Ok(())
}
