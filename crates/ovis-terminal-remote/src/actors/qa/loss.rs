use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum};
use serde_json::json;

use crate::actors::{ActorError, Inbound, Request};
use crate::qa_support::{HANG, TestResult, at, limits, machine, pipe, served};

#[tokio::test]
async fn a_lost_line_keeps_the_machine_listed_with_its_last_seen_state() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, mut inbound, peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let mut peer_0 = peers.into_iter().next().ok_or("peer")?;
    let next = |inbound: &mut tokio::sync::mpsc::Receiver<Inbound>| {
        let received = inbound.try_recv();
        received.map_err(|_| "no inbound record")
    };
    let mut received = Vec::new();
    received.push(tokio::time::timeout(HANG, inbound.recv()).await?);
    assert_eq!(received, vec![Some(Inbound::Attached(machine(0xb2)))]);
    let event = json!({"jsonrpc": "2.0", "method": "event", "params": {
        "seq": "5", "runtime_lifetime": "7", "ts_ms": "1", "type": "terminal.created",
        "terminal": {"terminal_id": "terminal:1", "revision": "2", "removed": false},
        "data": {},
    }});
    peer_0.write(&event).await?;
    let notified = tokio::time::timeout(HANG, inbound.recv()).await?;
    assert!(
        matches!(notified, Some(Inbound::Notification { ref method, .. }) if method == "event")
    );
    let revision_before = actors.list().first().map(|record| record.revision);

    drop(peer_0);
    assert_eq!(
        tokio::time::timeout(HANG, inbound.recv()).await?,
        Some(Inbound::Lost(machine(0xb2)))
    );
    let roster = actors.list();
    assert_eq!(roster.len(), 1, "an unreachable machine is never omitted");
    let listed = roster.first().ok_or("bravo listed")?;
    assert_eq!(listed.state, OpenEnum::Known(MachineState::Unreachable));
    assert_eq!(listed.last_seen_seq.map(EventSeq::get), Some(5));
    assert!(Some(listed.revision) > revision_before);
    match actors
        .call(&bravo, Request::new("server.status", json!({}), HANG))
        .await
    {
        Err(ActorError::Unreachable { last_seen_seq, .. }) => {
            assert_eq!(last_seen_seq.map(EventSeq::get), Some(5));
        }
        other => return Err(format!("expected machine_unreachable, got {other:?}").into()),
    }

    let (line, mut peer) = pipe(4096);
    actors.attach(&bravo, line).await?;
    assert_eq!(
        tokio::time::timeout(HANG, inbound.recv()).await?,
        Some(Inbound::Attached(machine(0xb2)))
    );
    let call = actors.call(&bravo, Request::new("server.status", json!({}), HANG));
    let (answer, _) = tokio::join!(call, peer.answer(json!({"back": true})));
    assert_eq!(answer?, Ok(json!({"back": true})));
    let health = actors.health(&bravo)?;
    assert_eq!(health.line_epoch.get(), 2);
    assert_eq!(health.state, OpenEnum::Known(MachineState::Connected));
    assert_eq!(health.last_seen_seq.map(EventSeq::get), Some(5));
    assert!(next(&mut inbound).is_err(), "nothing else was delivered");
    Ok(())
}

#[tokio::test]
async fn remote_errors_pass_through_and_remote_requests_are_refused() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let mut peer_0 = peers.into_iter().next().ok_or("peer")?;
    let call = actors.call(&bravo, Request::new("terminal.get", json!({}), HANG));
    let peer = async {
        let request = peer_0.next().await?;
        let error = json!({"code": -32000, "message": "m", "data": {"kind": "not_found"}});
        peer_0
            .write(&json!({"jsonrpc": "2.0", "id": at(&request, "/id"), "error": error}))
            .await?;
        peer_0
            .write(&json!({"jsonrpc": "2.0", "id": "p1", "method": "terminal.kill", "params": {}}))
            .await?;
        peer_0.next().await
    };
    let (answer, refusal) = tokio::join!(call, peer);
    let remote_error = answer?
        .err()
        .ok_or("the remote error is kept as an error")?;
    assert_eq!(at(&remote_error, "/data/kind"), "not_found");
    let refusal = refusal?;
    assert_eq!(at(&refusal, "/id"), "p1");
    assert_eq!(at(&refusal, "/error/data/kind"), "method_not_found");
    Ok(())
}
