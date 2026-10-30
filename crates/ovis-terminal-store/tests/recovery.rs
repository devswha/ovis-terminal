//! Explicit rebuild after quarantine keeps the recovery evidence queryable for good.

#[path = "support/store_fixture.rs"]
mod store_fixture;

use std::fs;
use std::path::Path;

use ovis_terminal_store::{
    CasWrite, Payload, RecoveryCause, Revision, Store, StoreOptions, Version,
    read_quarantine_evidence,
};
use store_fixture::{Fallible, TempRoot, key, set};

fn seed_and_corrupt(root: &Path) -> Fallible {
    let store = Store::open(root, StoreOptions::default())?;
    let write = CasWrite::new(key("a"), Version::ABSENT, Some(Payload::from("a")));
    store.submit(set("op-1", write))?.wait()?;
    store.close()?;
    let db = root.join("state.db");
    let mut bytes = fs::read(&db)?;
    bytes
        .get_mut(..16)
        .ok_or("database shorter than its header")?
        .fill(0xff);
    fs::write(&db, bytes)?;
    Ok(())
}

#[test]
fn rebuild_carries_the_quarantine_evidence_into_the_new_database() {
    let root = TempRoot::new("rebuild").unwrap();
    seed_and_corrupt(root.path()).unwrap();
    let quarantined = Store::open(root.path(), StoreOptions::default()).unwrap_err();
    let evidence = quarantined.recovery_evidence().unwrap().clone();

    let rebuilt = Store::rebuild_after_quarantine(root.path(), StoreOptions::default()).unwrap();
    rebuilt.close().unwrap();

    let reopened = Store::open(root.path(), StoreOptions::default()).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision(), Revision::ZERO);
    assert_eq!(
        snapshot.recovery_evidence().unwrap(),
        vec![evidence.clone()]
    );
    assert_eq!(snapshot.recovery_evidence().unwrap(), vec![evidence]);
}

#[test]
fn rebuild_refuses_a_root_with_nothing_quarantined() {
    let root = TempRoot::new("rebuild-empty").unwrap();

    let error = Store::rebuild_after_quarantine(root.path(), StoreOptions::default()).unwrap_err();

    assert_eq!(error.kind(), "nothing_to_rebuild");
    assert!(!root.path().join("state.db").exists());
}

#[test]
fn rebuild_leaves_a_newer_schema_in_place() {
    let root = TempRoot::new("rebuild-newer").unwrap();
    Store::open(root.path(), StoreOptions::default())
        .unwrap()
        .close()
        .unwrap();
    let db = root.path().join("state.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .pragma_update(None, "user_version", 42)
        .unwrap();
    let original = fs::read(&db).unwrap();

    let error = Store::rebuild_after_quarantine(root.path(), StoreOptions::default()).unwrap_err();

    assert_eq!(error.kind(), "store_unsupported", "{error:?}");
    let cause = &error.recovery_evidence().unwrap().cause;
    assert_eq!(
        *cause,
        RecoveryCause::UnsupportedSchema {
            found: 42,
            supported: ovis_terminal_store::SCHEMA_VERSION
        }
    );
    assert_eq!(fs::read(&db).unwrap(), original);
    assert_eq!(read_quarantine_evidence(root.path()).unwrap().len(), 1);
}
