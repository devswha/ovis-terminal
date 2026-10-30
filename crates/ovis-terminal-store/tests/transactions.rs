//! Commit atomicity across a killed writer process, compare-and-swap, and typed
//! rejection of damaged or newer databases. The reopened durable state is the oracle.

#[path = "support/store_fixture.rs"]
mod store_fixture;

use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus};

use ovis_terminal_store::{
    CasWrite, CommitBarrier, EventPage, MaterialDisposition, OpId, Payload, ProcessSurvival,
    ReceiptLookup, RecoveryCause, RelaunchPolicy, ResourceLookup, Revision, Store, StoreOptions,
    Version, read_quarantine_evidence,
};
use store_fixture::{Fallible, TempRoot, key, set};

const CHILD_ROOT: &str = "OVIS_STORE_CHILD_ROOT";
const CHILD_BARRIER: &str = "OVIS_STORE_CHILD_BARRIER";
const BARRIER_MARKER: &str = "barrier-reached";

fn create_doc() -> ovis_terminal_store::Mutation {
    set(
        "op-1",
        CasWrite::new(key("app"), Version::ABSENT, Some(Payload::from("v1"))),
    )
}

/// Runs `crash_child` in a separate process that aborts when the writer reaches `barrier`.
fn run_crashing_child(root: &Path, barrier: &str) -> Fallible<ExitStatus> {
    let status = Command::new(std::env::current_exe()?)
        .args(["--exact", "crash_child", "--ignored", "--nocapture"])
        .env(CHILD_ROOT, root)
        .env(CHILD_BARRIER, barrier)
        .status()?;
    if !root.join(BARRIER_MARKER).exists() {
        return Err(format!("child never reached {barrier}: {status}").into());
    }
    Ok(status)
}

#[test]
#[ignore = "child process fixture started by `happy` and `crash_before_commit_keeps_nothing`"]
fn crash_child() {
    let root = std::env::var_os(CHILD_ROOT).unwrap();
    let wanted = match std::env::var(CHILD_BARRIER).unwrap().as_str() {
        "after_commit" => CommitBarrier::Committed,
        "before_commit" => CommitBarrier::Applied,
        other => panic!("unknown barrier {other}"),
    };
    let marker = Path::new(&root).join(BARRIER_MARKER);
    let options = StoreOptions::default().with_barrier(move |reached| {
        if reached == wanted {
            fs::write(&marker, b"1").unwrap();
            std::process::abort();
        }
    });
    let store = Store::open(Path::new(&root), options).unwrap();
    let outcome = store.submit(create_doc()).unwrap().wait();
    panic!("the writer must abort at its barrier, got {outcome:?}");
}

#[test]
fn happy() {
    // given: a writer process killed right after COMMIT of resource + receipt + event
    let root = TempRoot::new("happy").unwrap();
    assert!(
        !run_crashing_child(root.path(), "after_commit")
            .unwrap()
            .success()
    );

    // when: the store is reopened by a new process lifetime
    let store = Store::open(root.path(), StoreOptions::default()).unwrap();
    let snapshot = store.snapshot().unwrap();

    // then: state, receipt and event are all present at revision 1
    let one = Revision::new(1);
    assert_eq!(snapshot.revision(), one);
    let live = ResourceLookup::Live {
        version: Version::new(1),
        value: Payload::from("v1"),
        revision: one,
    };
    assert_eq!(snapshot.resource(&key("app")).unwrap(), live);
    let receipt = ReceiptLookup::Found {
        revision: one,
        result: Payload::from("op-1"),
    };
    assert_eq!(snapshot.receipt(&OpId::new("op-1")).unwrap(), receipt);
    let EventPage::Events(events) = snapshot.events_after(Revision::ZERO, 10).unwrap() else {
        panic!("events below the retention floor");
    };
    let [event] = events.as_slice() else {
        panic!("expected exactly one event, got {events:?}")
    };
    assert_eq!((event.seq, event.op.as_str()), (one, "op-1"));
    println!(
        "reopened after crash: revision={} resource={live:?} receipt={receipt:?}",
        snapshot.revision()
    );
    drop(snapshot);

    // when: one CAS at the current version, then a second CAS with the same stale version
    let next = CasWrite::new(key("app"), Version::new(1), Some(Payload::from("v2")));
    let committed = store.submit(set("op-2", next)).unwrap().wait().unwrap();
    let stale = CasWrite::new(key("app"), Version::new(1), Some(Payload::from("v3")));
    let conflict = store
        .submit(set("op-3", stale))
        .unwrap()
        .wait()
        .unwrap_err();

    // then: value and version changed exactly once and the loser left no receipt
    assert_eq!(committed.revision, Revision::new(2));
    assert_eq!(conflict.kind(), "conflict");
    store.close().unwrap();
    let reopened = Store::open(root.path(), StoreOptions::default()).unwrap();
    let after = reopened.snapshot().unwrap();
    let two = Revision::new(2);
    let current = ResourceLookup::Live {
        version: Version::new(2),
        value: Payload::from("v2"),
        revision: two,
    };
    assert_eq!(after.resource(&key("app")).unwrap(), current);
    assert_eq!(
        after.receipt(&OpId::new("op-3")).unwrap(),
        ReceiptLookup::Absent
    );
    assert_eq!(after.revision(), two);
    println!(
        "cas: committed={committed:?} conflict.kind={} resource={current:?}",
        conflict.kind()
    );
}

#[test]
fn crash_before_commit_keeps_nothing() {
    let root = TempRoot::new("before-commit").unwrap();
    assert!(
        !run_crashing_child(root.path(), "before_commit")
            .unwrap()
            .success()
    );

    let store = Store::open(root.path(), StoreOptions::default()).unwrap();
    let snapshot = store.snapshot().unwrap();

    assert_eq!(snapshot.revision(), Revision::ZERO);
    assert_eq!(
        snapshot.resource(&key("app")).unwrap(),
        ResourceLookup::Absent
    );
    assert_eq!(
        snapshot.receipt(&OpId::new("op-1")).unwrap(),
        ReceiptLookup::Absent
    );
    assert_eq!(
        snapshot.events_after(Revision::ZERO, 10).unwrap(),
        EventPage::Events(Vec::new())
    );
}

#[derive(Debug, Clone, Copy)]
enum Damage {
    Header,
    FutureSchema,
}

fn seed_then_damage(root: &Path, damage: Damage) -> Fallible {
    let store = Store::open(root, StoreOptions::default())?;
    store.submit(create_doc())?.wait()?;
    store.close()?;
    let db = root.join("state.db");
    match damage {
        Damage::Header => {
            let mut bytes = fs::read(&db)?;
            bytes
                .get_mut(..16)
                .ok_or("database shorter than its header")?
                .copy_from_slice(b"not a database!\0");
            fs::write(&db, bytes)?;
        }
        Damage::FutureSchema => {
            rusqlite::Connection::open(&db)?.pragma_update(None, "user_version", 999)?;
        }
    }
    Ok(())
}

#[test]
fn failure() {
    for damage in [Damage::Header, Damage::FutureSchema] {
        // given: a store holding one committed document, then damaged on disk
        let root = TempRoot::new("failure").unwrap();
        seed_then_damage(root.path(), damage).unwrap();
        let damaged = fs::read(root.path().join("state.db")).unwrap();

        // when: the store is opened
        let error = Store::open(root.path(), StoreOptions::default()).unwrap_err();

        // then: a typed rejection names the preserved material and forbids relaunch
        let evidence = error.recovery_evidence().unwrap().clone();
        let (kind, cause, disposition) = match damage {
            Damage::Header => (
                "store_corrupted",
                RecoveryCause::CorruptHeader,
                MaterialDisposition::MovedToQuarantine,
            ),
            Damage::FutureSchema => (
                "store_unsupported",
                RecoveryCause::UnsupportedSchema {
                    found: 999,
                    supported: ovis_terminal_store::SCHEMA_VERSION,
                },
                MaterialDisposition::CopiedOriginalKept,
            ),
        };
        assert_eq!(error.kind(), kind, "{error:?}");
        assert_eq!(evidence.cause, cause);
        assert_eq!(evidence.flags.material, disposition);
        assert_eq!(evidence.flags.process_survival, ProcessSurvival::Unknown);
        assert_eq!(
            evidence.flags.relaunch,
            RelaunchPolicy::RequiresIndependentEvidence
        );
        assert_eq!(
            fs::read(evidence.location.join("state.db")).unwrap(),
            damaged
        );
        println!(
            "{damage:?}: kind={} hint={} evidence={evidence:?}",
            error.kind(),
            error.hint()
        );

        // then: reopening is still a typed rejection, never an empty healthy store
        let again = Store::open(root.path(), StoreOptions::default()).unwrap_err();
        assert_eq!(again.kind(), kind);

        // then: reading the evidence twice does not consume it
        let first_read = read_quarantine_evidence(root.path()).unwrap();
        assert_eq!(first_read, read_quarantine_evidence(root.path()).unwrap());
        assert!(first_read.contains(&evidence));
    }
}
