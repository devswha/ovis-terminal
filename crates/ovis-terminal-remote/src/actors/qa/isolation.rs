use std::time::Duration;

use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::json;
use tokio::io::AsyncReadExt as _;

use crate::actors::{ActorError, Actors, Limits, Request};
use crate::qa_support::{HANG, TestResult, at, limits, machine, pipe, served};

fn ping(deadline: Duration) -> Request {
    Request::new("server.status", json!({}), deadline)
}

#[tokio::test]
async fn a_stalled_machine_holds_neither_another_machine_nor_the_roster() -> TestResult {
    let (alpha, bravo) = (machine(0xa1), machine(0xb2));
    let (actors, _inbound, peers) = served(limits(), &[(0xa1, "alpha"), (0xb2, "bravo")]).await?;
    let mut peers = peers.into_iter();
    let (mut peer_0, mut peer_1) = (peers.next().ok_or("peer")?, peers.next().ok_or("peer")?);
    let actors = std::sync::Arc::new(actors);
    let stalled = {
        let actors = std::sync::Arc::clone(&actors);
        tokio::spawn(async move {
            actors
                .call(&machine(0xb2), ping(Duration::from_secs(60)))
                .await
        })
    };
    let bravo_request = peer_1.next().await?;
    assert_eq!(at(&bravo_request, "/method"), "server.status");

    let alpha = actors.call(&alpha, ping(HANG));
    let (answer, request) = tokio::join!(alpha, peer_0.answer(json!({"from": "alpha"})));
    assert_eq!(answer?, Ok(json!({"from": "alpha"})));
    assert_eq!(at(&request?, "/method"), "server.status");
    assert!(
        !stalled.is_finished(),
        "bravo's call is still waiting on bravo alone"
    );
    let roster = actors.list();
    assert_eq!(roster.len(), 2);
    assert!(
        roster
            .iter()
            .all(|record| record.state == OpenEnum::Known(MachineState::Connected))
    );
    assert_eq!(actors.health(&bravo)?.in_flight.get(), 1);

    drop(peer_1);
    let lost = tokio::time::timeout(HANG, stalled).await??;
    assert_eq!(
        lost.map_err(|error| error.kind()),
        Err("machine_unreachable")
    );
    Ok(())
}

#[tokio::test]
async fn an_expired_call_is_typed_counted_and_cancelled_on_the_peer() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let mut peer_0 = peers.into_iter().next().ok_or("peer")?;
    let deadline = Duration::from_millis(50);
    let expired = actors.call(&bravo, ping(deadline)).await;
    match expired {
        Err(ActorError::DeadlineExceeded {
            deadline: reported, ..
        }) => assert_eq!(reported, deadline),
        other => return Err(format!("expected deadline_exceeded, got {other:?}").into()),
    }
    let request = peer_0.next().await?;
    let cancel = peer_0.next().await?;
    assert_eq!(at(&cancel, "/method"), "$/cancelRequest");
    assert_eq!(at(&cancel, "/params/id"), at(&request, "/id"));
    let health = actors.health(&bravo)?;
    assert_eq!(health.deadline_exceeded.get(), 1);
    assert_eq!(health.in_flight.get(), 0);
    assert_eq!(health.state, OpenEnum::Known(MachineState::Connected));
    assert_eq!(
        health
            .last_failure
            .map(|failure| failure.kind.token().to_owned()),
        Some("deadline_exceeded".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn a_caller_that_leaves_cancels_its_call_on_the_peer() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let mut peer_0 = peers.into_iter().next().ok_or("peer")?;
    let actors = std::sync::Arc::new(actors);
    let leaving = {
        let actors = std::sync::Arc::clone(&actors);
        tokio::spawn(async move {
            actors
                .call(&machine(0xb2), ping(Duration::from_secs(60)))
                .await
        })
    };
    let first = peer_0.next().await?;
    leaving.abort();
    assert!(leaving.await.is_err_and(|error| error.is_cancelled()));
    let second = actors.call(&bravo, ping(HANG));
    let peer = async {
        let request = peer_0.next().await?;
        let cancel = peer_0.next().await?;
        let reply = json!({"jsonrpc": "2.0", "id": at(&request, "/id"), "result": {}});
        peer_0.write(&reply).await?;
        Ok::<_, Box<dyn std::error::Error>>(cancel)
    };
    let (answer, cancel) = tokio::join!(second, peer);
    assert_eq!(answer?, Ok(json!({})));
    let cancel = cancel?;
    assert_eq!(at(&cancel, "/method"), "$/cancelRequest");
    assert_eq!(at(&cancel, "/params/id"), at(&first, "/id"));
    Ok(())
}

#[tokio::test]
async fn a_hung_write_fills_only_that_machines_queue_and_then_loses_its_line() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound) = Actors::new(Limits::new(1, Duration::from_secs(60), 8));
    actors.add(crate::qa_support::record(0xb2, "bravo")?)?;
    let (line, mut peer) = pipe(16);
    actors.attach(&bravo, line).await?;
    let actors = std::sync::Arc::new(actors);
    let call = |deadline| {
        let actors = std::sync::Arc::clone(&actors);
        tokio::spawn(async move { actors.call(&machine(0xb2), ping(deadline)).await })
    };
    let hung = call(Duration::from_millis(300));
    let mut first = [0_u8; 1];
    tokio::time::timeout(HANG, peer.reader.read_exact(&mut first)).await??;
    let mut queued = std::pin::pin!(actors.call(&bravo, ping(HANG)));
    let polled_once = tokio::time::timeout(Duration::ZERO, &mut queued).await;
    assert!(
        polled_once.is_err(),
        "the queued call waits behind the hung write"
    );
    match actors.call(&bravo, ping(HANG)).await {
        Err(ActorError::QueueFull { capacity, .. }) => assert_eq!(capacity, 1),
        other => return Err(format!("expected backpressure, got {other:?}").into()),
    }
    let hung = tokio::time::timeout(HANG, hung).await??;
    assert_eq!(hung.map_err(|error| error.kind()), Err("deadline_exceeded"));
    let queued = tokio::time::timeout(HANG, queued).await?;
    assert_eq!(
        queued.map_err(|error| error.kind()),
        Err("machine_unreachable")
    );
    let health = actors.health(&bravo)?;
    assert_eq!(health.state, OpenEnum::Known(MachineState::Unreachable));
    Ok(())
}

#[tokio::test]
async fn refusals_are_typed_before_anything_is_sent() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, _peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let unknown = actors.call(&machine(0xc3), ping(HANG)).await;
    assert_eq!(unknown.map_err(|error| error.kind()), Err("not_found"));
    for deadline in [Duration::ZERO, Duration::from_secs(61)] {
        let refused = actors.call(&bravo, ping(deadline)).await;
        assert_eq!(refused.map_err(|error| error.kind()), Err("invalid_params"));
    }
    let twice = actors.add(crate::qa_support::record(0xb2, "bravo")?);
    assert_eq!(twice.map_err(|error| error.kind()), Err("machine_exists"));
    assert_eq!(actors.health(&bravo)?.in_flight.get(), 0);
    Ok(())
}
