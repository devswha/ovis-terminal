//! Orphan delivery over real actor lines to a peer that receives with [`Inbox`]; the
//! outbox here is an in-memory store with the durable outbox's contract.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ovis_terminal_protocol::domain::agent_tree::OrphanNotice;
use ovis_terminal_protocol::domain::machine_actor::{
    AckOutcome, NotificationId, OrphanedAckParams, OrphanedParams, ParentIncarnation,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::json;

mod memory;

use memory::Memory;

use super::{Acknowledgements, Inbox, Outbox, OutboxError, replay};
use crate::qa_support::{HANG, Peer, TestResult, at, limits, machine, pipe, served};

fn notice(incarnation: u64) -> Result<OrphanNotice, serde_json::Error> {
    serde_json::from_value(json!({
        "parent_machine": machine(0x44),
        "parent": {"terminal_id": "terminal:6", "incarnation": incarnation.to_string()},
        "finished_seq": incarnation.to_string(),
    }))
}

fn outbox(notices: Vec<OrphanNotice>) -> Memory {
    Memory(Mutex::new(BTreeMap::from([(machine(0xb2), notices)])))
}

/// Receives one `$/orphaned` with `inbox`, counting applications in `applied`, and
/// answers it unless `answer` is false; returns the notice's incarnation.
async fn receive(
    peer: &mut Peer,
    (inbox, applied): (&Inbox, &AtomicU64),
    answer: bool,
) -> Result<u64, Box<dyn std::error::Error>> {
    let request = peer.next().await?;
    assert_eq!(at(&request, "/method"), "$/orphaned");
    let params: OrphanedParams = serde_json::from_value(at(&request, "/params"))?;
    let result = inbox
        .receive(&params, |_| async {
            applied.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
        .await?;
    if answer {
        peer.write(&json!({"jsonrpc": "2.0", "id": at(&request, "/id"), "result": result}))
            .await?;
    }
    Ok(params.notice.parent.incarnation.get())
}

#[tokio::test]
async fn notices_replay_in_order_one_at_a_time_until_acknowledged() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let mut peer_0 = peers.into_iter().next().ok_or("peer")?;
    let store = outbox(vec![notice(1)?, notice(2)?]);
    let (acks, inbox, applied) = (
        Acknowledgements::default(),
        Inbox::new(machine(0xb2)),
        AtomicU64::new(0),
    );
    let sender = replay((&actors, &store), &acks, (&bravo, HANG));
    let peer = async {
        let first = receive(&mut peer_0, (&inbox, &applied), true).await?;
        let second = receive(&mut peer_0, (&inbox, &applied), true).await?;
        Ok::<_, Box<dyn std::error::Error>>((first, second))
    };
    let (replayed, order) = tokio::join!(sender, peer);
    let replayed = replayed?;
    assert_eq!(
        order?,
        (1, 2),
        "an older incarnation's death is delivered first"
    );
    assert_eq!((replayed.applied.get(), replayed.remaining.get()), (2, 0));
    assert!(replayed.stopped.is_none());
    assert_eq!(store.pending(&bravo).await?, Vec::<OrphanNotice>::new());
    Ok(())
}

#[tokio::test]
async fn a_lost_acknowledgement_is_replayed_without_applying_twice() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let store = outbox(vec![notice(1)?]);
    let (acks, inbox, applied) = (
        Acknowledgements::default(),
        Inbox::new(machine(0xb2)),
        AtomicU64::new(0),
    );
    let mut peer = peers.into_iter().next().ok_or("peer")?;
    let sender = replay((&actors, &store), &acks, (&bravo, HANG));
    let applies_then_drops = async {
        receive(&mut peer, (&inbox, &applied), false).await?;
        drop(peer);
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let (lost, dropped) = tokio::join!(sender, applies_then_drops);
    dropped?;
    let lost = lost?;
    assert_eq!(
        lost.stopped.map(|error| error.kind()),
        Some("machine_unreachable")
    );
    assert_eq!(store.pending(&bravo).await?.len(), 1, "the outbox keeps it");

    let (line, mut peer) = pipe(4096);
    actors.attach(&bravo, line).await?;
    let sender = replay((&actors, &store), &acks, (&bravo, HANG));
    let (replayed, received) = tokio::join!(sender, receive(&mut peer, (&inbox, &applied), true));
    received?;
    let replayed = replayed?;
    assert_eq!((replayed.duplicates.get(), replayed.applied.get()), (1, 0));
    assert_eq!(applied.load(Ordering::SeqCst), 1, "remote work ran once");
    assert_eq!(store.pending(&bravo).await?, Vec::<OrphanNotice>::new());
    Ok(())
}

#[tokio::test]
async fn a_frozen_machine_keeps_its_notice_pending() -> TestResult {
    let bravo = machine(0xb2);
    let (actors, _inbound, _peers) = served(limits(), &[(0xb2, "bravo")]).await?;
    let store = outbox(vec![notice(1)?]);
    let acks = Acknowledgements::default();
    let frozen = replay(
        (&actors, &store),
        &acks,
        (&bravo, Duration::from_millis(50)),
    )
    .await?;
    assert_eq!(
        frozen.stopped.map(|error| error.kind()),
        Some("deadline_exceeded")
    );
    assert_eq!(frozen.remaining.get(), 1);
    assert_eq!(store.pending(&bravo).await?, vec![notice(1)?]);
    Ok(())
}

#[tokio::test]
async fn acknowledgements_by_id_refuse_unknown_and_stale_without_touching_the_outbox() -> TestResult
{
    let bravo = machine(0xb2);
    let store = outbox(vec![notice(2)?]);
    let acks = Acknowledgements::default();
    let machines = [machine(0xa1), machine(0xb2)];
    let ack = |id: NotificationId, parent: ParentIncarnation| {
        serde_json::from_value::<OrphanedAckParams>(
            json!({"notification_id": id, "parent_incarnation": parent}),
        )
    };
    let old_token: ParentIncarnation = serde_json::from_value(json!("reused-old"))?;
    let unknown: NotificationId = serde_json::from_value(json!("unknown"))?;
    let id = NotificationId::of(&bravo, &notice(2)?);
    let refused = acks
        .acknowledge((&store, &machines), &ack(unknown, old_token.clone())?)
        .await;
    assert_eq!(
        refused.map_err(|error| error.kind()).err(),
        Some("not_found")
    );
    let stale = acks
        .acknowledge((&store, &machines), &ack(id.clone(), old_token.clone())?)
        .await;
    assert_eq!(
        stale.map_err(|error| error.kind()).err(),
        Some("stale_target")
    );
    let older = ParentIncarnation::of(&notice(1)?);
    let earlier = acks
        .acknowledge((&store, &machines), &ack(id.clone(), older)?)
        .await;
    assert_eq!(
        earlier.map_err(|error| error.kind()).err(),
        Some("stale_target")
    );
    assert_eq!(
        store.pending(&bravo).await?.len(),
        1,
        "refusals change nothing"
    );

    let right = ParentIncarnation::of(&notice(2)?);
    let settled = acks
        .acknowledge((&store, &machines), &ack(id.clone(), right.clone())?)
        .await?;
    assert_eq!(settled.outcome, OpenEnum::Known(AckOutcome::Acknowledged));
    assert_eq!(store.pending(&bravo).await?, Vec::<OrphanNotice>::new());
    let again = acks
        .acknowledge((&store, &machines), &ack(id.clone(), right)?)
        .await?;
    assert_eq!(again.outcome, OpenEnum::Known(AckOutcome::Duplicate));
    let stale_again = acks
        .acknowledge((&store, &machines), &ack(id, old_token)?)
        .await;
    assert_eq!(
        stale_again.map_err(|error| error.kind()).err(),
        Some("stale_target")
    );
    Ok(())
}

#[tokio::test]
async fn an_inbox_refuses_a_notice_addressed_elsewhere_without_applying_it() -> TestResult {
    let (alpha, bravo) = (machine(0xa1), machine(0xb2));
    let inbox = Inbox::new(machine(0xb2));
    let elsewhere = OrphanedParams::to(&alpha, notice(1)?);
    let refused = inbox
        .receive(&elsewhere, |_| async {
            Err(OutboxError::new(("internal", "x"), "must not run"))
        })
        .await;
    assert_eq!(
        refused.map_err(|error| error.kind()).err(),
        Some("stale_target")
    );
    let here = OrphanedParams::to(&bravo, notice(1)?);
    let first = inbox.receive(&here, |_| async { Ok(true) }).await?;
    let replay = inbox
        .receive(&here, |_| async {
            Err(OutboxError::new(("internal", "x"), "must not run"))
        })
        .await?;
    assert_eq!(at(&serde_json::to_value(first)?, "/delivery"), "applied");
    assert_eq!(at(&serde_json::to_value(replay)?, "/delivery"), "duplicate");
    Ok(())
}
