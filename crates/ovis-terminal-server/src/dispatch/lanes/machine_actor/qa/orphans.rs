use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ovis_terminal_agent::tree::Link;
use ovis_terminal_protocol::domain::agent_tree::OrphanNotice;
use ovis_terminal_protocol::domain::machine_actor::{
    NotificationId, OrphanedParams, ParentIncarnation,
};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::terminal::ChildPolicy;
use ovis_terminal_remote::actors::Inbound;
use ovis_terminal_remote::orphans::{Inbox, Outbox as _};
use serde_json::{Value, json};

use super::{HANG, Harness, LOCAL, Peer, TestResult, at, bravo, harness, parent_ref, pipe};
use crate::dispatch::Read as _;
use crate::dispatch::lanes::machine_actor::{MachineList, TreeOutbox};

fn roster(
    h: &Harness,
    list: &MachineList<super::PipeLine, super::AgentStatusEvent, super::OperationProducer>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let params: NoParams = serde_json::from_value(json!({}))?;
    let read = tokio::task::block_in_place(|| {
        h.core.read_view(|snapshot| {
            Ok::<_, ovis_terminal_core::events::EventError>(list.read(params, snapshot))
        })
    })?;
    Ok(read.map_err(|error| error.kind().to_owned())?)
}

/// Receives one `$/orphaned` with `inbox`, counting applications, and answers it.
async fn answer(peer: &mut Peer, (inbox, applied): (&Inbox, &AtomicU64)) -> TestResult {
    let request = peer.next().await?;
    assert_eq!(at(&request, "/method"), "$/orphaned");
    let params: OrphanedParams = serde_json::from_value(at(&request, "/params"))?;
    let result = inbox
        .receive(&params, |_| async {
            applied.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
        .await?;
    peer.write(&json!({"jsonrpc": "2.0", "id": at(&request, "/id"), "result": result}))
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_frozen_machine_keeps_its_orphan_until_it_returns_and_acknowledges() -> TestResult {
    let h = harness()?;
    let parent = h.launch(6, 1)?;
    let first = parent_ref(6, 1)?;
    tokio::task::block_in_place(|| {
        h.run(|snapshot| h.tree.link_remote(snapshot, (first, bravo())))
    })?;
    let exit = tokio::task::block_in_place(|| h.exited(parent))?;
    let notice: OrphanNotice = exit
        .deliveries
        .first()
        .map(|(_, notice)| notice.clone())
        .ok_or("one delivery")?;
    let (lane, mut inbound) = h.lane(Duration::from_millis(100))?;
    let list = MachineList::new(&lane);
    let (frozen_end, frozen) = pipe();
    lane.actors().attach(&bravo(), frozen_end).await?;

    let stalled = lane.deliver(&bravo()).await?;
    assert_eq!(
        stalled.stopped.map(|error| error.kind()),
        Some("deadline_exceeded")
    );
    assert_eq!(stalled.remaining.get(), 1);
    let unknown = lane
        .acknowledge(json!({"notification_id": "unknown", "parent_incarnation": "reused-old"}))
        .await;
    assert_eq!(
        unknown
            .map_err(|error| error.kind().to_owned())
            .err()
            .as_deref(),
        Some("not_found")
    );
    let id = NotificationId::of(&bravo(), &notice);
    let stale = lane
        .acknowledge(json!({"notification_id": id, "parent_incarnation": "reused-old"}))
        .await;
    assert_eq!(
        stale
            .map_err(|error| error.kind().to_owned())
            .err()
            .as_deref(),
        Some("stale_target")
    );
    let numeric = lane
        .acknowledge(json!({"notification_id": id, "parent_incarnation": 1}))
        .await;
    assert_eq!(
        numeric
            .map_err(|error| error.kind().to_owned())
            .err()
            .as_deref(),
        Some("invalid_params")
    );
    let outbox = TreeOutbox::new(
        std::sync::Arc::clone(&h.tree),
        std::sync::Arc::clone(&h.core),
    );
    assert_eq!(
        outbox.pending(&bravo()).await?,
        vec![notice.clone()],
        "refusals keep the outbox"
    );

    drop(frozen);
    loop {
        match tokio::time::timeout(HANG, inbound.recv()).await? {
            Some(Inbound::Lost(machine)) if machine == bravo() => break,
            Some(_) => {}
            None => return Err("the actors stopped".into()),
        }
    }
    let listed = roster(&h, &list)?;
    assert_eq!(at(&listed, "/machines/0/machine_id"), json!(bravo()));
    assert_eq!(at(&listed, "/machines/0/state"), "unreachable");

    let (inbox, applied, machine) = (Inbox::new(bravo()), AtomicU64::new(0), bravo());
    let (fresh_end, mut peer) = pipe();
    let (replayed, answered) = tokio::join!(
        lane.attach(&machine, fresh_end),
        answer(&mut peer, (&inbox, &applied))
    );
    answered?;
    let replayed = replayed?;
    assert_eq!((replayed.applied.get(), replayed.remaining.get()), (1, 0));
    assert!(replayed.stopped.is_none());
    assert_eq!(applied.load(Ordering::SeqCst), 1);
    assert_eq!(outbox.pending(&bravo()).await?, Vec::<OrphanNotice>::new());
    assert_eq!(at(&roster(&h, &list)?, "/machines/0/state"), "connected");
    let token = ParentIncarnation::of(&notice);
    let again = lane
        .acknowledge(json!({"notification_id": id, "parent_incarnation": token}))
        .await;
    assert_eq!(
        at(&again.map_err(|error| error.kind().to_owned())?, "/outcome"),
        "duplicate"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_received_death_orders_ends_once_and_refuses_another_machines_notice() -> TestResult {
    let h = harness()?;
    let child = h.launch(7, 1)?;
    let parent = parent_ref(6, 2)?;
    tokio::task::block_in_place(|| {
        h.run(|snapshot| {
            let link = Link::new(parent.clone(), child, ChildPolicy::new(true, true));
            Ok(h.tree.link_to_remote(snapshot, (bravo(), link))?.writes)
        })
    })?;
    let notice: OrphanNotice = serde_json::from_value(json!({
        "parent_machine": bravo(), "parent": parent, "finished_seq": "9",
    }))?;
    let (lane, _inbound) = h.lane(HANG)?;
    let here = OrphanedParams::to(&MachineId::new(LOCAL.to_owned()), notice.clone());
    let (applied, orders) = lane.orphaned(&here).await?;
    assert_eq!(at(&serde_json::to_value(&applied)?, "/delivery"), "applied");
    assert_eq!(orders, vec![child]);
    let (replayed, none) = lane.orphaned(&here).await?;
    assert_eq!(
        at(&serde_json::to_value(&replayed)?, "/delivery"),
        "duplicate"
    );
    assert_eq!(none, Vec::new(), "a replayed notice ends nothing again");
    let elsewhere = OrphanedParams::to(&bravo(), notice);
    let refused = lane.orphaned(&elsewhere).await;
    assert_eq!(
        refused.map_err(|error| error.kind()).err(),
        Some("stale_target")
    );
    Ok(())
}
