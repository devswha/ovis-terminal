//! The mirror against a real coordinator and store: local order is commit order, remote
//! order and clocks are evidence only.

use std::path::PathBuf;

use ovis_terminal_core::events::{
    Coordinator, EventError, Filter, RemoteOrigin, RemoteSeq, SnapshotProducer,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId as CoreMachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::domain::machine_actor::{RemoteEvent, RemoteScan};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum, TimestampMs};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde_json::json;

use super::{Mirror, MirrorEvent, Mirrored};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug)]
struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _: &Snapshot, _: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

fn machine(last: u8) -> MachineId {
    MachineId::new(format!(
        "machine:0191f1a2-0000-7000-8000-0000000000{last:02x}"
    ))
}

fn event(
    seq: u64,
    (lifetime, ts_ms): (&str, u64),
    terminal: Option<(&str, u64, bool)>,
) -> Result<RemoteEvent, serde_json::Error> {
    let terminal = terminal.map(|(id, revision, removed)| {
        json!({"terminal_id": id, "revision": revision.to_string(), "removed": removed})
    });
    serde_json::from_value(json!({
        "seq": seq.to_string(), "runtime_lifetime": lifetime, "ts_ms": ts_ms.to_string(),
        "type": "terminal.updated", "terminal": terminal, "data": {"seq": seq},
    }))
}

fn kinds(records: &[Mirrored]) -> Vec<String> {
    records
        .iter()
        .map(|record| match record.record() {
            MirrorEvent::Mirrored(event) => format!("event:{}", event.source.seq.get()),
            MirrorEvent::Boundary(boundary) => match &boundary.kind {
                OpenEnum::Known(kind) => format!("{kind:?}"),
                OpenEnum::Unknown(kind) => kind.clone(),
            },
        })
        .collect()
}

#[test]
fn local_order_follows_arrival_while_remote_order_and_clocks_stay_evidence() -> TestResult {
    let root = Root(std::env::temp_dir().join(format!("ovis-t44-{}", uuid::Uuid::now_v7())));
    std::fs::create_dir_all(&root.0)?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(
            "machine:0191f1a2-0000-7000-8000-000000000044".parse()?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(44)?,
        ServerBootId::try_from(1)?,
    );
    let core = Coordinator::<MirrorEvent, _>::new(
        Store::open(&root.0.join("state"), StoreOptions::default())?,
        identity,
        NoSections,
    );
    let subscription = core.subscribe(None)?;
    let mut mirror = Mirror::default();
    let local = TimestampMs::new(1_000_000);
    let ahead = 1_000_000 + 600_000;
    let mut records = mirror.accept(
        (&machine(0xb2), local),
        event(900, ("runtime:7", ahead), None)?,
    )?;
    records.extend(mirror.accept((&machine(0xa1), local), event(3, ("runtime:5", 10), None)?)?);
    let mut cursors = Vec::new();
    for record in records {
        cursors.push(core.commit(record.into_commit::<MirrorEvent>()?)?);
    }
    let (first, second) = (
        subscription.delivery.try_recv()?,
        subscription.delivery.try_recv()?,
    );
    assert!(
        first.cursor.seq < second.cursor.seq,
        "arrival order, not remote seq or time"
    );
    assert_eq!(Some(&first.cursor), cursors.first());
    let origin = RemoteOrigin::new(
        machine(0xb2).as_str().parse::<CoreMachineId>()?,
        RuntimeId::try_from(7)?,
        RemoteSeq::new(EventSeq::new(900)),
    );
    assert_eq!(first.origin, Some(origin));
    match &first.data {
        MirrorEvent::Mirrored(event) => {
            assert_eq!(event.clock_skew_ms.get(), 600_000);
            assert_eq!(event.source.seq.get(), 900);
            assert_eq!(event.source.machine_id, machine(0xb2));
        }
        MirrorEvent::Boundary(_) => return Err("expected the mirrored event".into()),
    }
    assert_eq!(second.origin.map(|origin| origin.seq.get()), Some(3));
    drop(subscription);
    core.close()?;
    Ok(())
}

#[test]
fn gaps_scans_losses_and_lifetimes_are_explicit_boundaries() -> TestResult {
    let mut mirror = Mirror::default();
    let bravo = machine(0xb2);
    let at = (&bravo, TimestampMs::new(5));
    let terminal = Some(("terminal:1", 4, false));
    assert_eq!(
        kinds(&mirror.accept(at, event(1, ("runtime:7", 5), terminal)?)?),
        ["event:1"]
    );
    assert_eq!(
        kinds(&mirror.accept(at, event(1, ("runtime:7", 5), None)?)?),
        Vec::<String>::new()
    );
    assert_eq!(
        kinds(&mirror.accept(at, event(4, ("runtime:7", 5), None)?)?),
        ["Gap", "event:4"]
    );
    let scan: RemoteScan =
        serde_json::from_value(json!({"runtime_lifetime": "runtime:7", "scan_seq": "9"}))?;
    assert_eq!(kinds(&mirror.scan(&bravo, &scan)?), ["Scan"]);
    assert_eq!(kinds(&mirror.scan(&bravo, &scan)?), Vec::<String>::new());
    assert_eq!(
        kinds(&mirror.accept(at, event(10, ("runtime:7", 5), None)?)?),
        ["event:10"]
    );

    let lost = mirror.lost(&bravo);
    assert_eq!(kinds(std::slice::from_ref(&lost)), ["LineLost"]);
    if let MirrorEvent::Boundary(boundary) = lost.record() {
        assert_eq!(
            boundary.last_seen.as_ref().map(|seen| seen.seq.get()),
            Some(10)
        );
    }
    let kept = mirror.terminals(&bravo);
    assert_eq!(kept.len(), 1, "an unreachable terminal is never omitted");
    assert!(
        kept.iter()
            .all(|state| !state.available && state.revision.get() == 4)
    );
    assert_eq!(kinds(&[mirror.resumed(&bravo)]), ["Resumed"]);

    let restarted = mirror.accept(at, event(1, ("runtime:8", 5), None)?)?;
    assert_eq!(kinds(&restarted), ["LifetimeChanged", "event:1"]);
    let removed = mirror.accept(
        at,
        event(2, ("runtime:8", 5), Some(("terminal:1", 5, true)))?,
    )?;
    assert_eq!(kinds(&removed), ["event:2"]);
    assert!(
        mirror.terminals(&bravo).is_empty(),
        "only a remote removal forgets a terminal"
    );
    Ok(())
}

#[test]
fn a_lifetime_that_is_no_runtime_identity_changes_nothing() -> TestResult {
    let mut mirror = Mirror::default();
    let bravo = machine(0xb2);
    let refused = mirror.accept(
        (&bravo, TimestampMs::new(1)),
        event(1, ("not-a-lifetime", 1), None)?,
    );
    assert_eq!(
        refused.map_err(|error| error.kind()).err(),
        Some("remote_incompatible")
    );
    assert_eq!(
        kinds(&mirror.accept(
            (&bravo, TimestampMs::new(1)),
            event(1, ("runtime:7", 1), None)?
        )?),
        ["event:1"]
    );
    Ok(())
}
