//! The dedicated writer: bounded admission, pinned read snapshots and async completion.

#[path = "support/store_fixture.rs"]
mod store_fixture;

use std::sync::{Mutex, mpsc};
use std::time::Duration;

use ovis_terminal_store::{
    CasWrite, CommitBarrier, Payload, ResourceLookup, Revision, Store, StoreLimits, StoreOptions,
    Version,
};
use store_fixture::{TempRoot, key, set};

const BOUND: Duration = Duration::from_secs(10);

fn create(op: &str, id: &str) -> ovis_terminal_store::Mutation {
    set(
        op,
        CasWrite::new(key(id), Version::ABSENT, Some(Payload::from(id))),
    )
}

#[test]
fn a_full_writer_queue_rejects_with_backpressure() {
    let root = TempRoot::new("backpressure").unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let options = StoreOptions::default()
        .with_limits(StoreLimits::default().with_writer_queue(1))
        .with_barrier(move |reached| {
            if reached == CommitBarrier::Applied {
                entered_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv_timeout(BOUND).unwrap();
            }
        });
    let store = Store::open(root.path(), options).unwrap();
    let running = store.submit(create("op-1", "a")).unwrap();
    entered_rx.recv_timeout(BOUND).unwrap();
    let queued = store.submit(create("op-2", "b")).unwrap();

    let rejected = store.submit(create("op-3", "c")).unwrap_err();

    assert_eq!(rejected.kind(), "backpressure");
    release_tx.send(()).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(running.wait().unwrap().revision, Revision::new(1));
    assert_eq!(queued.wait().unwrap().revision, Revision::new(2));
}

#[test]
fn a_snapshot_stays_at_its_revision_while_later_commits_land() {
    let root = TempRoot::new("pinned").unwrap();
    let store = Store::open(root.path(), StoreOptions::default()).unwrap();
    store.submit(create("op-1", "a")).unwrap().wait().unwrap();
    let pinned = store.snapshot().unwrap();

    let update = CasWrite::new(key("a"), Version::new(1), Some(Payload::from("a2")));
    store.submit(set("op-2", update)).unwrap().wait().unwrap();

    assert_eq!(pinned.revision(), Revision::new(1));
    let old = ResourceLookup::Live {
        version: Version::new(1),
        value: Payload::from("a"),
        revision: Revision::new(1),
    };
    assert_eq!(pinned.resource(&key("a")).unwrap(), old);
    assert_eq!(store.snapshot().unwrap().revision(), Revision::new(2));
}

#[tokio::test]
async fn a_ticket_resolves_on_an_async_runtime() {
    let root = TempRoot::new("async").unwrap();
    let store = Store::open(root.path(), StoreOptions::default()).unwrap();

    let ticket = store.submit(create("op-1", "a")).unwrap();

    let receipt = tokio::time::timeout(BOUND, ticket.outcome())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.revision, Revision::new(1));
}

#[test]
fn an_oversized_value_is_rejected_before_it_is_queued() {
    let root = TempRoot::new("too-large").unwrap();
    let options =
        StoreOptions::default().with_limits(StoreLimits::default().with_max_value_bytes(4));
    let store = Store::open(root.path(), options).unwrap();
    let oversized = CasWrite::new(key("a"), Version::ABSENT, Some(Payload::from("12345")));

    let error = store.submit(set("op-1", oversized)).unwrap_err();

    assert_eq!(error.kind(), "too_large");
    assert_eq!(store.snapshot().unwrap().revision(), Revision::ZERO);
}
