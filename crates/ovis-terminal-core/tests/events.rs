//! Commit-order and snapshot-boundary behavior against the real SQLite writer.

use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use ovis_terminal_core::events::{
    Commit, Coordinator, EventData, EventError, Filter, MissedReason, ResourceId, Section,
    SnapshotProducer, SnapshotSections,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, IdentityError, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
    TerminalId,
};
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_store::{
    CasWrite, CommitBarrier, OpId, Payload, ResourceKey, ResourceLookup, Snapshot, Store,
    StoreError, StoreOptions, Version,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

#[path = "../src/events/qa/contracts.rs"]
mod contracts;
#[path = "../src/events/qa/durable.rs"]
mod durable;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Changed {
    value: String,
}

impl EventData for Changed {
    fn event_type(&self) -> &'static str {
        "terminal.updated"
    }
}

#[derive(Debug)]
struct Producer;

impl SnapshotProducer for Producer {
    type Sections = SnapshotSections<(), Vec<String>>;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Self::Sections, EventError> {
        let rows = match snapshot.resource(&ResourceKey::new("terminal", "terminal:1"))? {
            ResourceLookup::Live { value, .. } => {
                vec![serde_json::from_slice::<Changed>(value.as_bytes())?.value]
            }
            ResourceLookup::Absent | ResourceLookup::Tombstone { .. } => Vec::new(),
            _ => return Err(EventError::SnapshotUnavailable),
        };
        Ok(SnapshotSections::new(
            Section::Unavailable,
            Section::Complete(rows),
        ))
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("ovis-events-{}", Uuid::now_v7())))
    }

    fn open(&self, options: StoreOptions) -> Result<Store, StoreError> {
        Store::open(&self.0, options)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "events fixture cleanup: {removed:?}");
    }
}

fn identity(boot: u64) -> Result<RuntimeIdentity, IdentityError> {
    Ok(RuntimeIdentity::new(
        HostIdentity::new(
            MachineId::new(Uuid::from_u128(1))?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(1)?,
        ServerBootId::try_from(boot)?,
    ))
}

fn change(value: &str, version: u64) -> Result<Commit<Changed>, Box<dyn Error>> {
    let data = Changed {
        value: value.into(),
    };
    let write = CasWrite::new(
        ResourceKey::new("terminal", "terminal:1"),
        Version::new(version),
        Some(Payload::from(serde_json::to_vec(&data)?)),
    );
    Ok(Commit::new(
        OpId::new(Uuid::now_v7().to_string()),
        data,
        vec![ResourceId::Terminal(TerminalId::try_from(1)?)],
    )
    .write(write))
}

#[test]
fn happy() -> TestResult {
    // Given a writer paused immediately before COMMIT.
    let fixture = Fixture::new();
    let (applied, reached) = mpsc::sync_channel(1);
    let release = Arc::new(Barrier::new(2));
    let writer_release = Arc::clone(&release);
    let options = StoreOptions::default().with_barrier(move |point| {
        if matches!(point, CommitBarrier::Applied) {
            assert!(applied.send(()).is_ok());
            writer_release.wait();
        }
    });
    let core = Arc::new(Coordinator::new(
        fixture.open(options)?,
        identity(1)?,
        Producer,
    ));
    let before = core.subscribe(None)?;
    // When a subscription races that transaction.
    let mutation_core = Arc::clone(&core);
    let mutation = change("one", 0)?;
    let commit = std::thread::spawn(move || mutation_core.commit(mutation));
    reached.recv_timeout(Duration::from_secs(5))?;
    assert!(
        matches!(
            before.delivery.try_recv(),
            Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
        ),
        "no publication before commit"
    );
    let subscribing_core = Arc::clone(&core);
    let (started, start) = mpsc::sync_channel(1);
    let subscription = std::thread::spawn(move || {
        assert!(started.send(()).is_ok());
        subscribing_core.subscribe(None)
    });
    start.recv_timeout(Duration::from_secs(5))?;
    release.wait();
    let receipt = commit.join().map_err(|_| "commit thread panicked")??;
    let racing = subscription
        .join()
        .map_err(|_| "subscription thread panicked")??;
    // Then snapshot R plus delivery after R reconstructs the mutation exactly once.
    let event = before.delivery.recv_timeout(Duration::from_secs(5))?;
    assert_eq!(event.cursor.seq, receipt.seq);
    let snapshot = racing.snapshot.as_ref().ok_or("fresh snapshot missing")?;
    let mut reconstructed = match &snapshot.sections.terminals {
        Section::Complete(rows) => rows.clone(),
        Section::Unavailable => return Err("terminal snapshot unavailable".into()),
        _ => return Err("unknown terminal snapshot availability".into()),
    };
    if snapshot.cursor.seq < receipt.seq {
        let change = racing.delivery.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(change.cursor, receipt);
        reconstructed.push(change.data.value);
    } else {
        assert_eq!(snapshot.cursor.seq, receipt.seq);
    }
    assert_eq!(reconstructed, vec!["one".to_owned()]);
    assert!(matches!(
        racing.delivery.try_recv(),
        Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
    ));
    assert!(snapshot.cursor.seq <= event.cursor.seq);
    println!("{}", serde_json::to_string(&event)?);
    println!("{}", serde_json::to_string(&racing.snapshot)?);
    Arc::try_unwrap(core)
        .map_err(|_| "core still shared")?
        .close()?;
    Ok(())
}

#[test]
fn failure() -> TestResult {
    // Given a committed state and a snapshot page that is explicitly released.
    let fixture = Fixture::new();
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    );
    let committed = core.commit(change("one", 0)?)?;
    let pinned = core.pin_snapshot()?;
    core.release_snapshot(&pinned.token)?;
    // When a cursor from another runtime and a stale page token are submitted.
    let mut foreign = committed;
    foreign.runtime_id = RuntimeId::try_from(2)?;
    let recovered = core.subscribe(Some(foreign))?;
    let stale = core.page(&pinned.token, ByteOffset::new(0));
    // Then recovery carries a fresh snapshot and paging rejects the stale token.
    assert_eq!(recovered.missed, Some(MissedReason::LifetimeMismatch));
    let snapshot = recovered
        .snapshot
        .as_ref()
        .ok_or("recovery snapshot missing")?;
    assert_eq!(
        snapshot.sections.terminals,
        Section::Complete(vec!["one".into()])
    );
    assert!(matches!(stale, Err(error) if error.kind() == "stale_snapshot"));
    let wrong_version = core.commit(change("conflicting", 0)?);
    assert!(matches!(wrong_version, Err(error) if error.kind() == "conflict"));
    assert!(matches!(
        recovered.delivery.try_recv(),
        Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
    ));
    println!("missed=lifetime_mismatch; page.kind=stale_snapshot; write.kind=conflict");
    core.close()?;
    Ok(())
}
