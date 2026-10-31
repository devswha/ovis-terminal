//! Safe socket paths, side-effect-free probes and the published error kinds.

use std::fs::DirBuilder;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_protocol::schema::error_kinds::DomainErrorKind;
use ovis_terminal_server::listen::{EndpointStatus, Probe, UnixEndpoint};
use serde_json::json;

use super::handler::Recorder;
use super::support::{Fixture, TestResult, WAIT, at, error_kind, request};

#[tokio::test]
async fn a_live_socket_is_never_taken_over() -> TestResult {
    let fixture = Fixture::new()?;
    let server = fixture.serve(1, Arc::new(Recorder::default())).await?;
    let mut owner = fixture.connect().await?;
    let ready = owner.hello("1", &fixture.bearer).await?;
    assert!(
        ready.get("result").is_some(),
        "the live server answered hello"
    );

    let contested = UnixEndpoint::bind(&fixture.path("s.sock"))
        .await
        .err()
        .ok_or("bound a live path twice")?;
    assert_eq!(contested.kind(), "already_running");

    owner
        .send(&request("after", "doc.get", &json!({ "name": "b" })))
        .await?;
    assert_eq!(at(&owner.expect_next().await?, "/result/name"), "b");
    let mut late = fixture.connect().await?;
    let reply = late.hello("1", &fixture.bearer).await?;
    assert!(
        reply.get("result").is_some(),
        "the server outlived the probe"
    );
    drop((owner, late));
    let report = server.shutdown().await?;
    assert_eq!((report.accepted(), report.panicked()), (3, 0));
    Ok(())
}

#[tokio::test]
async fn a_server_that_hangs_up_mid_hello_is_unreachable() -> TestResult {
    let fixture = Fixture::new()?;
    let rude = fixture.path("rude.sock");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(rude.parent().ok_or("parent")?)?;
    let listener = tokio::net::UnixListener::bind(&rude)?;
    let hang_up = tokio::spawn(async move { listener.accept().await.map(drop) });
    let observed = Probe::new(&fixture.bearer, WAIT).observe(&rude).await?;
    assert_eq!(observed, EndpointStatus::Unreachable);
    hang_up.await??;
    Ok(())
}

#[tokio::test]
async fn a_stale_socket_is_replaced_and_a_plain_file_never_is() -> TestResult {
    let fixture = Fixture::new()?;
    let stale = fixture.path("stale.sock");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(stale.parent().ok_or("parent")?)?;
    drop(std::os::unix::net::UnixListener::bind(&stale)?);
    let rebound = UnixEndpoint::bind(&stale).await?;
    rebound.remove()?;
    assert!(!stale.exists());

    let marker = fixture.path("marker.sock");
    std::fs::write(&marker, b"not a socket")?;
    let refused = UnixEndpoint::bind(&marker)
        .await
        .err()
        .ok_or("bound a file")?;
    assert_eq!(refused.kind(), "not_a_socket");
    let observed = Probe::new(&fixture.bearer, WAIT).observe(&marker).await;
    assert_eq!(observed?, EndpointStatus::StaleEndpoint);
    assert_eq!(std::fs::read(&marker)?, b"not a socket");
    Ok(())
}

#[tokio::test]
async fn a_directory_other_accounts_can_enter_is_refused() -> TestResult {
    let fixture = Fixture::new()?;
    let socket = fixture.path("s.sock");
    let directory = socket.parent().ok_or("parent")?;
    DirBuilder::new().recursive(true).create(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755))?;
    let refused = UnixEndpoint::bind(&socket)
        .await
        .err()
        .ok_or("bound insecurely")?;
    assert_eq!(refused.kind(), "endpoint_insecure");
    assert!(!socket.exists(), "a refused bind leaves no socket behind");
    Ok(())
}

#[tokio::test]
async fn a_bound_socket_and_its_directory_are_private() -> TestResult {
    let fixture = Fixture::new()?;
    let endpoint = UnixEndpoint::bind(&fixture.path("s.sock")).await?;
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).map(|m| m.permissions().mode() & 0o777);
    assert_eq!(mode(endpoint.path())?, 0o600);
    assert_eq!(mode(endpoint.path().parent().ok_or("parent")?)?, 0o700);
    endpoint.remove()?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_socket_that_never_answers_hello_is_unreachable() -> TestResult {
    let fixture = Fixture::new()?;
    let silent = fixture.path("silent.sock");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(silent.parent().ok_or("parent")?)?;
    let listener = std::os::unix::net::UnixListener::bind(&silent)?;
    let probe = Probe::new(&fixture.bearer, Duration::from_secs(30));
    assert_eq!(probe.observe(&silent).await?, EndpointStatus::Unreachable);
    drop(listener);
    Ok(())
}

#[tokio::test]
async fn a_rejected_probe_reports_the_typed_refusal() -> TestResult {
    let fixture = Fixture::new()?;
    let server = fixture.serve(1, Arc::new(Recorder::default())).await?;
    let probe = Probe::new("c1.bogus", WAIT);
    let refused = probe
        .observe(&fixture.path("s.sock"))
        .await
        .err()
        .ok_or("accepted a bogus bearer")?;
    assert_eq!(refused.kind(), "hello_rejected");
    let mut client = fixture.connect().await?;
    assert_eq!(error_kind(&client.hello("1", "").await?), "unauthenticated");
    drop(client);
    server.shutdown().await?;
    Ok(())
}

#[test]
fn every_kind_the_hello_boundary_emits_is_published() {
    let emitted = [
        "unauthenticated",
        "invalid_credential",
        "credential_expired",
        "credential_revoked",
        "decision_mismatch",
        "grant_exceeds_issuer",
        "endpoint_insecure",
        "peer_rejected",
        "unsupported_generation",
        "backpressure",
    ];
    for kind in emitted {
        let published: Result<DomainErrorKind, _> = serde_json::from_value(kind.into());
        assert!(published.is_ok(), "{kind} is in the published catalog");
    }
}
