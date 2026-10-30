//! Compare-and-swap commits all of a mutation or none of it, against a reference model.

#[path = "support/store_fixture.rs"]
mod store_fixture;

use std::collections::HashMap;

use ovis_terminal_store::{
    CasWrite, EventKind, EventPage, NewEvent, OpId, Payload, ReceiptLookup, ResourceLookup,
    Revision, Store, StoreOptions, Version,
};
use proptest::prelude::*;
use store_fixture::{TempRoot, key, set};

#[test]
fn a_conflicting_write_rolls_back_the_whole_mutation() {
    let root = TempRoot::new("all-or-nothing").unwrap();
    let store = Store::open(root.path(), StoreOptions::default()).unwrap();
    let fresh = CasWrite::new(key("a"), Version::ABSENT, Some(Payload::from("a")));
    let stale = CasWrite::new(key("b"), Version::new(7), Some(Payload::from("b")));

    let error = store
        .submit(set("op-1", fresh).compare_and_swap(stale))
        .unwrap()
        .wait()
        .unwrap_err();

    assert_eq!(error.kind(), "conflict");
    let snapshot = store.snapshot().unwrap();
    assert_eq!(
        snapshot.resource(&key("a")).unwrap(),
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

#[test]
fn a_reused_operation_id_is_rejected_without_a_second_commit() {
    let root = TempRoot::new("reused-op").unwrap();
    let store = Store::open(root.path(), StoreOptions::default()).unwrap();
    let first = CasWrite::new(key("a"), Version::ABSENT, Some(Payload::from("a")));
    store.submit(set("op-1", first)).unwrap().wait().unwrap();

    let second = CasWrite::new(key("b"), Version::ABSENT, Some(Payload::from("b")));
    let error = store
        .submit(set("op-1", second))
        .unwrap()
        .wait()
        .unwrap_err();

    assert_eq!(error.kind(), "duplicate_operation");
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.revision(), Revision::new(1));
    assert_eq!(
        snapshot.resource(&key("b")).unwrap(),
        ResourceLookup::Absent
    );
}

#[test]
fn events_of_one_mutation_take_consecutive_sequence_numbers() {
    let root = TempRoot::new("two-events").unwrap();
    let store = Store::open(root.path(), StoreOptions::default()).unwrap();
    let write = CasWrite::new(key("a"), Version::ABSENT, Some(Payload::from("a")));
    let second = NewEvent::new(EventKind::new("doc.audited"), Payload::from("audit"));

    let receipt = store
        .submit(set("op-1", write).event(second))
        .unwrap()
        .wait()
        .unwrap();

    assert_eq!(
        (receipt.first_event, receipt.revision),
        (Revision::new(1), Revision::new(2))
    );
    let EventPage::Events(events) = store
        .snapshot()
        .unwrap()
        .events_after(Revision::ZERO, 10)
        .unwrap()
    else {
        panic!("nothing was evicted");
    };
    let kinds: Vec<&str> = events.iter().map(|event| event.kind.as_str()).collect();
    assert_eq!(kinds, vec!["doc.changed", "doc.audited"]);
}

#[derive(Debug, Clone)]
struct Attempt {
    doc: usize,
    expected: u64,
    value: Option<u8>,
}

fn attempts() -> impl Strategy<Value = Vec<Attempt>> {
    let attempt = (0usize..3, 0u64..4, proptest::option::of(any::<u8>())).prop_map(
        |(doc, expected, value)| Attempt {
            doc,
            expected,
            value,
        },
    );
    proptest::collection::vec(attempt, 1..12)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn versions_follow_the_reference_model(sequence in attempts()) {
        let root = TempRoot::new("model").unwrap();
        let store = Store::open(root.path(), StoreOptions::default()).unwrap();
        let mut model: HashMap<usize, (u64, Option<u8>)> = HashMap::new();

        for (index, attempt) in sequence.iter().enumerate() {
            let id = format!("doc-{}", attempt.doc);
            let value = attempt.value.map(|byte| Payload::from(vec![byte]));
            let write = CasWrite::new(key(&id), Version::new(attempt.expected), value);
            let outcome = store.submit(set(&format!("op-{index}"), write)).unwrap().wait();
            let current = model.get(&attempt.doc).map_or(0, |(version, _)| *version);
            if attempt.expected == current {
                prop_assert!(outcome.is_ok());
                model.insert(attempt.doc, (current.checked_add(1).unwrap(), attempt.value));
            } else {
                prop_assert_eq!(outcome.unwrap_err().kind(), "conflict");
            }
        }

        let snapshot = store.snapshot().unwrap();
        for doc in 0..3 {
            let found = snapshot.resource(&key(&format!("doc-{doc}"))).unwrap();
            let expected = model.get(&doc).map(|(version, byte)| (Version::new(*version), byte.map(|byte| Payload::from(vec![byte]))));
            prop_assert_eq!(version_and_value(found), expected);
        }
    }
}

/// The compare-and-swap state of a lookup, without the revision that wrote it.
fn version_and_value(found: ResourceLookup) -> Option<(Version, Option<Payload>)> {
    match found {
        ResourceLookup::Live { version, value, .. } => Some((version, Some(value))),
        ResourceLookup::Tombstone { version, .. } => Some((version, None)),
        _ => None,
    }
}
