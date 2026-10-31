//! Authenticated persistent endpoints over a real Unix socket (plan todo 13).
//!
//! `happy` and `failure` are the QA entry points named by the plan; the modules hold
//! the malformed-input, stale-state and cancel/resume classes.
#![cfg(unix)]

#[path = "connection/adversarial.rs"]
mod adversarial;
#[path = "connection/bound.rs"]
mod bound;
#[path = "connection/descriptor.rs"]
mod descriptor;
#[path = "connection/endpoint.rs"]
mod endpoint;
#[path = "connection/handler.rs"]
pub mod handler;
#[path = "connection/support.rs"]
pub mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use handler::Recorder;
use ovis_terminal_protocol::schema::ids::MethodName;
use ovis_terminal_protocol::schema::methods::server::{HelloResult, ServerState};
use ovis_terminal_server::listen::{Continuity, EndpointIdentity, EndpointStatus, Probe};
use serde_json::{Value, json};
use support::{Fixture, TestResult, WAIT, at, error_kind, request};

fn hello_result(reply: &Value) -> TestResult<HelloResult> {
    Ok(serde_json::from_value(at(reply, "/result").clone())?)
}

fn identity(reply: &Value) -> TestResult<EndpointIdentity> {
    Ok(EndpointIdentity::of(&hello_result(reply)?))
}

async fn observe(fixture: &Fixture, path: &Path) -> TestResult<EndpointStatus> {
    Ok(Probe::new(&fixture.bearer, WAIT).observe(path).await?)
}

/// Direct children of this test process, as the operating system lists them.
fn child_processes() -> TestResult<usize> {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("pgrep")
        .args(["-P", &pid])
        .output()?;
    Ok(String::from_utf8(output.stdout)?.lines().count())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy() -> TestResult {
    let fixture = Fixture::new()?;
    let server = fixture.serve(1, Arc::new(Recorder::default())).await?;
    let mut client = fixture.connect().await?;
    let hello = client.hello_framed("1", &fixture.bearer).await?;
    let first = hello_result(&hello)?;
    assert_eq!(first.generation.get(), 1);
    let methods: BTreeSet<&str> = first.methods.iter().map(MethodName::as_str).collect();
    assert_eq!(
        methods,
        BTreeSet::from(["doc.get", "doc.set", "events.subscribe"])
    );

    client
        .send(&request("s1", "events.subscribe", &json!({})))
        .await?;
    let subscribed = client.expect_next().await?;
    assert_eq!(at(&subscribed, "/id"), "s1");
    let subscription = at(&subscribed, "/result/subscription_id").clone();

    client
        .send(&request("a", "doc.get", &json!({ "name": "a" })))
        .await?;
    client
        .send(&request("b", "doc.get", &json!({ "name": "b" })))
        .await?;
    client
        .send(&request("m", "doc.set", &json!({ "name": "x" })))
        .await?;
    let (mut order, mut events, mut replies) = (Vec::new(), Vec::new(), BTreeMap::new());
    while replies.len() < 3 || events.is_empty() {
        let message = client.expect_next().await?;
        if at(&message, "/method") == "event" {
            events.push(message);
        } else {
            let id = at(&message, "/id").as_str().unwrap_or_default().to_owned();
            order.push(id.clone());
            replies.insert(id, message);
        }
    }
    let reply = |id: &str| replies.get(id).cloned().unwrap_or_default();
    assert_eq!(at(&reply("a"), "/result/name"), "a");
    assert_eq!(at(&reply("b"), "/result/name"), "b");
    assert_eq!(at(&reply("m"), "/result/version"), "1");
    let position = |id: &str| order.iter().position(|seen| seen == id);
    assert!(position("b") < position("a"), "a waited for b: {order:?}");
    assert_eq!(events.len(), 1);
    let event = events.first().cloned().unwrap_or_default();
    assert_eq!(at(&event, "/params/subscription_id"), &subscription);
    assert_eq!(at(&event, "/params/seq"), "1");
    drop(client);

    let mut reconnected = fixture.connect().await?;
    let again = identity(&reconnected.hello("1", &fixture.bearer).await?)?;
    assert_eq!(again.continuity(&identity(&hello)?), Continuity::SameBoot);
    let probed = observe(&fixture, &fixture.path("s.sock")).await?;
    let EndpointStatus::Running(probed) = probed else {
        return Err(format!("a live server is running, observed {probed:?}").into());
    };
    assert_eq!(
        EndpointIdentity::of(&probed).continuity(&again),
        Continuity::SameBoot
    );
    drop(reconnected);
    let report = server.shutdown().await?;
    assert_eq!(
        (report.accepted(), report.panicked(), report.aborted()),
        (3, 0, 0)
    );
    assert!(report.failures().is_empty(), "{:?}", report.failures());
    let stopped = observe(&fixture, &fixture.path("s.sock")).await?;
    assert_eq!(stopped.state(), ServerState::Stopped);

    let restarted = fixture.serve(2, Arc::new(Recorder::default())).await?;
    let mut after_restart = fixture.connect().await?;
    let reply = after_restart.hello("1", &fixture.bearer).await?;
    assert_eq!(
        identity(&reply)?.continuity(&again),
        Continuity::ControlRestarted
    );
    let restarted_hello = hello_result(&reply)?;
    assert_eq!(restarted_hello.runtime_lifetime, first.runtime_lifetime);
    assert_ne!(restarted_hello.server_boot_id, first.server_boot_id);
    drop(after_restart);
    restarted.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure() -> TestResult {
    let fixture = Fixture::new()?;
    let recorder = Arc::new(Recorder::default());
    let server = fixture.serve(1, Arc::clone(&recorder)).await?;
    let mut client = fixture.connect().await?;
    let refused = client.hello_framed("999", &fixture.bearer).await?;
    assert_eq!(error_kind(&refused), "unsupported_generation");
    assert!(refused.get("result").is_none());
    client
        .send(&request("m1", "doc.set", &json!({ "name": "x" })))
        .await?;
    let blocked = client.expect_next().await?;
    assert_eq!(at(&blocked, "/id"), "m1");
    assert_eq!(error_kind(&blocked), "unauthenticated");
    assert_eq!((recorder.calls(), recorder.mutations()), (0, 0));

    let stale = fixture.path("stale.sock");
    drop(std::os::unix::net::UnixListener::bind(&stale)?);
    let before = child_processes()?;
    let status = observe(&fixture, &stale).await?;
    assert_eq!(status, EndpointStatus::StaleEndpoint);
    assert_eq!(status.state(), ServerState::StaleEndpoint);
    assert_eq!(child_processes()?, before);
    assert!(stale.exists(), "observing never removes the stale socket");
    drop(client);
    server.shutdown().await?;
    Ok(())
}
