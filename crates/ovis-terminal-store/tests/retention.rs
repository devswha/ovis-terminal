//! Eviction advances a visible floor or leaves a tombstone instead of an empty success.

#[path = "support/store_fixture.rs"]
mod store_fixture;

use ovis_terminal_store::{
    CasWrite, EventPage, OpId, Payload, ReceiptLookup, ResourceLookup, Revision, Store,
    StoreLimits, StoreOptions, Version,
};
use store_fixture::{Fallible, TempRoot, key, set};

fn store_with(root: &TempRoot, limits: StoreLimits) -> Fallible<Store> {
    Ok(Store::open(
        root.path(),
        StoreOptions::default().with_limits(limits),
    )?)
}

fn commit_three_docs(store: &Store) -> Fallible {
    for (op, id) in [("op-1", "a"), ("op-2", "b"), ("op-3", "c")] {
        let write = CasWrite::new(key(id), Version::ABSENT, Some(Payload::from(id)));
        store.submit(set(op, write))?.wait()?;
    }
    Ok(())
}

#[test]
fn evicted_events_report_the_floor_instead_of_an_empty_page() {
    let root = TempRoot::new("events-floor").unwrap();
    let store = store_with(&root, StoreLimits::default().with_max_events(2)).unwrap();
    commit_three_docs(&store).unwrap();

    let snapshot = store.snapshot().unwrap();

    let floor = Revision::new(2);
    assert_eq!(
        snapshot.events_after(Revision::ZERO, 10).unwrap(),
        EventPage::Expired { floor }
    );
    let EventPage::Events(retained) = snapshot.events_after(Revision::new(1), 10).unwrap() else {
        panic!("a cursor at the floor must still be served");
    };
    let seqs: Vec<Revision> = retained.iter().map(|event| event.seq).collect();
    assert_eq!(seqs, vec![Revision::new(2), Revision::new(3)]);
}

#[test]
fn evicted_receipts_are_reported_as_not_retained() {
    let root = TempRoot::new("receipt-floor").unwrap();
    let store = store_with(&root, StoreLimits::default().with_max_receipts(2)).unwrap();
    commit_three_docs(&store).unwrap();

    let snapshot = store.snapshot().unwrap();

    let floor = Revision::new(2);
    assert_eq!(
        snapshot.receipt(&OpId::new("op-1")).unwrap(),
        ReceiptLookup::NotRetained { floor }
    );
    assert_eq!(
        snapshot.receipt(&OpId::new("never-sent")).unwrap(),
        ReceiptLookup::NotRetained { floor }
    );
    let kept = ReceiptLookup::Found {
        revision: Revision::new(3),
        result: Payload::from("op-3"),
    };
    assert_eq!(snapshot.receipt(&OpId::new("op-3")).unwrap(), kept);
}

#[test]
fn deleting_a_resource_leaves_a_versioned_tombstone() {
    let root = TempRoot::new("tombstone").unwrap();
    let store = store_with(&root, StoreLimits::default()).unwrap();
    let create = CasWrite::new(key("app"), Version::ABSENT, Some(Payload::from("v1")));
    store.submit(set("op-1", create)).unwrap().wait().unwrap();

    let delete = CasWrite::new(key("app"), Version::new(1), None);
    store.submit(set("op-2", delete)).unwrap().wait().unwrap();

    let snapshot = store.snapshot().unwrap();
    let tombstone = ResourceLookup::Tombstone {
        version: Version::new(2),
        revision: Revision::new(2),
    };
    assert_eq!(snapshot.resource(&key("app")).unwrap(), tombstone);
    assert_eq!(
        snapshot.resource(&key("never-created")).unwrap(),
        ResourceLookup::Absent
    );
}

#[test]
fn recreating_after_a_tombstone_continues_the_version() {
    let root = TempRoot::new("recreate").unwrap();
    let store = store_with(&root, StoreLimits::default()).unwrap();
    let create = CasWrite::new(key("app"), Version::ABSENT, Some(Payload::from("v1")));
    store.submit(set("op-1", create)).unwrap().wait().unwrap();
    store
        .submit(set(
            "op-2",
            CasWrite::new(key("app"), Version::new(1), None),
        ))
        .unwrap()
        .wait()
        .unwrap();

    let again = CasWrite::new(key("app"), Version::new(2), Some(Payload::from("v3")));
    let receipt = store.submit(set("op-3", again)).unwrap().wait().unwrap();

    assert_eq!(receipt.versions, vec![(key("app"), Version::new(3))]);
}
