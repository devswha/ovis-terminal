//! Operation outcome, registry reservation, terminal state and replay form one unit.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ovis_terminal_core::events::{
    Cursor, EventData, EventError, Filter, ResourceId, SnapshotProducer,
};
use ovis_terminal_core::ops::{Completion, OperationSnapshot};
use ovis_terminal_core::registry::{IdentityReservation, TerminalId};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{
    CasWrite, CommitBarrier, Payload, ResourceKey, ResourceLookup, Snapshot, Version,
};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
enum UnitEvent {
    Operation(Box<OperationEvent>),
    Created(TerminalId),
}

impl From<OperationEvent> for UnitEvent {
    fn from(value: OperationEvent) -> Self {
        Self::Operation(Box::new(value))
    }
}

impl EventData for UnitEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Operation(event) => event.event_type(),
            Self::Created(_) => "terminal.created",
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Operation(event) => event.at_sequence(seq),
            Self::Created(_) => {}
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Sections {
    operations: OperationSnapshot,
    terminal: Option<TerminalId>,
}

impl AsRef<OperationSnapshot> for Sections {
    fn as_ref(&self) -> &OperationSnapshot {
        &self.operations
    }
}

#[derive(Debug)]
struct Producer(OperationProducer);

impl SnapshotProducer for Producer {
    type Sections = Sections;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Sections, EventError> {
        let terminal = match snapshot.resource(&key())? {
            ResourceLookup::Live { value, .. } => Some(serde_json::from_slice(value.as_bytes())?),
            ResourceLookup::Absent | ResourceLookup::Tombstone { .. } => None,
            _ => return Err(EventError::SnapshotUnavailable),
        };
        Ok(Sections {
            operations: self.0.capture(snapshot)?,
            terminal,
        })
    }
}

fn key() -> ResourceKey {
    ResourceKey::new("terminal", "atomic-fixture")
}

fn open(
    root: &Path,
    options: StoreOptions,
) -> Result<Coordinator<UnitEvent, Producer>, Box<dyn Error>> {
    let identity = identity(1)?;
    Ok(Coordinator::new(
        Store::open(root, options)?,
        identity,
        Producer(OperationProducer::new(identity.runtime_id())),
    ))
}

fn effects(
    snapshot: &Snapshot,
    version: Version,
) -> Result<Completion<UnitEvent>, ovis_terminal_core::ops::OperationError> {
    let (id, writes) = IdentityReservation::stage::<TerminalId>(snapshot)?.into_parts();
    let mut effect = Completion::new(serde_json::to_value(id)?);
    for write in writes {
        effect = effect.write(write);
    }
    Ok(effect
        .write(CasWrite::new(
            key(),
            version,
            Some(Payload::from(serde_json::to_vec(&id)?)),
        ))
        .event(UnitEvent::Created(id), vec![ResourceId::Terminal(id)]))
}

#[test]
fn failed_unit_preserves_running_receipt_and_reserves_no_identity() -> TestResult {
    // Given: durable operation intent, but no terminal or reserved identity.
    let fixture = Fixture::new();
    let core = open(&fixture.0, StoreOptions::default())?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("atomic")?;
    operations.accept(&req)?;
    let permit = operations.start(req.op_id())?;
    let observer = core.subscribe(None)?;
    let before = observer.snapshot.ok_or("snapshot")?.cursor;

    // When: reservation and receipt writes encounter a failing terminal CAS.
    let rejected = operations
        .commit_unit(permit, |snapshot| effects(snapshot, Version::new(1)))
        .expect_err("absent terminal cannot have version one");

    // Then: all parts roll back, including the identity allocator and event sequence.
    assert_eq!(rejected.kind(), "conflict");
    assert_eq!(
        operations.get(req.op_id())?.state,
        OpenEnum::Known(OpState::Running)
    );
    let snapshot = core.read_snapshot()?;
    assert_eq!(snapshot.cursor, before);
    assert_eq!(snapshot.sections.terminal, None);
    assert!(matches!(
        observer.delivery.try_recv(),
        Err(EventError::TryReceive(std::sync::mpsc::TryRecvError::Empty))
    ));
    core.close()?;
    let store = Store::open(&fixture.0, StoreOptions::default())?;
    let (next, _) = IdentityReservation::stage::<TerminalId>(&store.snapshot()?)?.into_parts();
    assert_eq!(next, TerminalId::try_from(1)?);
    store.close()?;
    Ok(())
}

#[test]
fn crash_after_unit_commit_replays_all_parts_before_any_publication() -> TestResult {
    const ROOT: &str = "OVIS_T16_ATOMIC_CRASH_ROOT";
    if let Some(path) = std::env::var_os(ROOT) {
        // Given in the child: abort only at the final unit's durable COMMIT barrier.
        let armed = Arc::new(AtomicBool::new(false));
        let enabled = Arc::clone(&armed);
        let marker = Path::new(&path).join("committed");
        let options = StoreOptions::default().with_barrier(move |point| {
            if point == CommitBarrier::Committed && enabled.load(Ordering::SeqCst) {
                std::fs::write(&marker, b"committed").expect("crash marker");
                std::process::abort();
            }
        });
        let core = open(Path::new(&path), options)?;
        let operations = Operations::new(&core, context(1)?);
        let req = request("atomic")?;
        operations.accept(&req)?;
        let permit = operations.start(req.op_id())?;
        armed.store(true, Ordering::SeqCst);
        operations.commit_unit(permit, |snapshot| effects(snapshot, Version::ABSENT))?;
        panic!("the real writer must abort before its commit reply");
    }
    // Given: an isolated store and a child using the production operation/coordinator path.
    let fixture = Fixture::new();
    let status = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "atomic::crash_after_unit_commit_replays_all_parts_before_any_publication",
            "--nocapture",
        ])
        .env(ROOT, &fixture.0)
        .status()?;
    assert!(!status.success());
    assert_eq!(std::fs::read(fixture.0.join("committed"))?, b"committed");

    // When: another control lifetime opens the actual committed database.
    let core = open(&fixture.0, StoreOptions::default())?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("atomic")?;
    let receipt = operations.get(req.op_id())?;
    let state = core.read_snapshot()?;
    let replay = core.subscribe(Some(Cursor::new(
        identity(1)?.machine_id(),
        identity(1)?.runtime_id(),
        EventSeq::new(2),
    )))?;

    // Then: the OpRecord, terminal, reservation, and both events survived together.
    assert_eq!(receipt.state, OpenEnum::Known(OpState::Committed));
    assert_eq!(state.sections.terminal, Some(TerminalId::try_from(1)?));
    assert_eq!(
        receipt.result,
        Some(serde_json::to_value(state.sections.terminal)?)
    );
    let operation = replay.delivery.try_recv()?;
    let created = replay.delivery.try_recv()?;
    assert_eq!(operation.event_type, "op.committed");
    assert_eq!(created.event_type, "terminal.created");
    assert_eq!(
        created.cursor.seq.get(),
        operation
            .cursor
            .seq
            .get()
            .checked_add(1)
            .ok_or("overflow")?
    );
    assert_eq!(state.cursor, created.cursor);
    core.close()?;
    let store = Store::open(&fixture.0, StoreOptions::default())?;
    assert!(
        IdentityReservation::new(&store)
            .is_reserved(&store.snapshot()?, TerminalId::try_from(1)?)?
    );
    store.close()?;
    Ok(())
}
