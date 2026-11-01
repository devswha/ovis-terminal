use std::sync::Arc;

use ovis_terminal_core::events::{Coordinator, EventError, Filter, SnapshotProducer};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_remote::mirror::MirrorEvent;
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde_json::json;

use super::{TestResult, bravo, harness, pipe};
use crate::dispatch::lanes::machine_actor::pump;

#[derive(Debug)]
struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _: &Snapshot, _: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

fn label(event: &MirrorEvent) -> String {
    match event {
        MirrorEvent::Mirrored(event) => format!("event:{}", event.source.seq.get()),
        MirrorEvent::Boundary(boundary) => match &boundary.kind {
            OpenEnum::Known(kind) => format!("{kind:?}"),
            OpenEnum::Unknown(kind) => kind.clone(),
        },
        _ => "unknown".to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_machine_line_is_mirrored_with_explicit_resume_and_loss_boundaries() -> TestResult {
    let h = harness()?;
    let store = Store::open(&h.root.join("mirror"), StoreOptions::default())?;
    let core = Arc::new(Coordinator::<MirrorEvent, _>::new(
        store, h.identity, NoSections,
    ));
    let subscription = core.subscribe(None)?;
    let (lane, inbound) = h.lane(super::HANG)?;
    let pumping = tokio::spawn(pump(inbound, Arc::clone(&core)));
    let (local_end, mut peer) = pipe();
    lane.actors().attach(&bravo(), local_end).await?;
    peer.write(&json!({"jsonrpc": "2.0", "method": "event", "params": {
        "seq": "41", "runtime_lifetime": "runtime:7", "ts_ms": "1", "type": "terminal.created",
        "terminal": null, "data": {},
    }}))
    .await?;
    peer.write(&json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 42}}))
        .await?;
    drop(peer);
    let events = tokio::task::spawn_blocking(move || {
        let received: Result<Vec<_>, EventError> = (0..3)
            .map(|_| subscription.delivery.recv_timeout(super::HANG))
            .collect();
        received
    })
    .await??;
    drop(lane);
    let report = tokio::time::timeout(super::HANG, pumping).await??;
    assert_eq!((report.committed, report.refused, report.failed), (3, 1, 0));
    let labels: Vec<String> = events.iter().map(|event| label(&event.data)).collect();
    assert_eq!(labels, ["Resumed", "event:41", "LineLost"]);
    assert!(
        events
            .windows(2)
            .all(|pair| matches!(pair, [before, after] if before.cursor.seq < after.cursor.seq))
    );
    let mirrored = events.get(1).ok_or("the mirrored event")?;
    assert_eq!(
        mirrored.origin.as_ref().map(|origin| origin.seq.get()),
        Some(41)
    );
    Arc::try_unwrap(core)
        .map_err(|_| "coordinator still shared")?
        .close()?;
    Ok(())
}
