//! The store keeps what the engine still shows, bounded by its cap, oldest out first.

use std::sync::Arc;

use ovis_terminal_protocol::domain::graphics::{
    GraphicsCaps, GraphicsProtocol, GraphicsReleaseReason, GraphicsReleased,
};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset};
use ovis_terminal_vt::{Geometry, HistoryLimit, ImageSet, OutputOffset, Vt, VtConfig};

use super::{ImageStore, RELEASED_LIMIT, ReleasedUnit};

fn image(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=T,f=100,i={id};AAAAAAAA\x1b\\").into_bytes()
}

/// An engine that drew `count` images on successive rows, and their keys.
fn drawn(count: u32) -> (Vt, Vec<OutputOffset>) {
    let geometry = Geometry::new(20, 8).expect("valid geometry");
    let mut vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(10)));
    let mut keys = Vec::new();
    for id in 1..=count {
        keys.push(vt.offset());
        drop(vt.feed(&image(id)));
        drop(vt.feed(b"\r\n"));
    }
    (vt, keys)
}

fn store(cap: u64) -> ImageStore {
    let mut limits = Limits::v1();
    limits.graphics_terminal_store_max_bytes = ByteCount::new(cap);
    ImageStore::new(GraphicsCaps::advertised(&limits))
}

fn unit(key: OutputOffset, id: u32) -> ReleasedUnit {
    ReleasedUnit::new(key, GraphicsProtocol::Kitty, Arc::from(image(id)))
}

fn size() -> u64 {
    u64::try_from(image(1).len()).expect("small")
}

fn released(store: &ImageStore) -> Vec<(u64, GraphicsReleaseReason)> {
    store
        .released()
        .iter()
        .map(|item: &GraphicsReleased| {
            let reason = serde_json::from_value(serde_json::to_value(&item.reason).expect("enum"))
                .expect("known reason");
            (item.output_offset.get(), reason)
        })
        .collect()
}

#[test]
fn only_images_the_engine_retains_are_admitted() {
    let (vt, keys) = drawn(1);
    let mut store = store(1024);
    let stray = OutputOffset::new(999);
    assert_eq!(store.admit(unit(stray, 9), &vt.images()), None);
    assert!(store.pieces(stray).is_none());
    let key = keys.first().copied().expect("drawn");
    assert_eq!(store.admit(unit(key, 1), &vt.images()), None);
    let pieces = store.pieces(key).expect("admitted");
    assert_eq!(pieces.concat(), image(1), "kept byte-identical");
}

#[test]
fn at_the_cap_the_oldest_is_evicted_first_with_typed_counts() {
    let (vt, keys) = drawn(3);
    let shown = vt.images();
    let cap = size() * 2 + 1;
    let mut store = store(cap);
    let mut notices = Vec::new();
    for (id, key) in (1..).zip(&keys) {
        notices.push(store.admit(unit(*key, id), &shown));
    }
    let notice = notices.pop().flatten().expect("the third admission evicts");
    assert_eq!(notices, [None, None]);
    assert_eq!(notice.evicted_sequences.get(), 1);
    assert_eq!(notice.evicted_bytes.get(), size());
    assert_eq!(notice.retained_bytes.get(), size() * 2);
    assert_eq!(notice.limit_bytes.get(), cap);
    assert_eq!(store.retained_bytes(), size() * 2);
    let first = keys.first().copied().expect("drawn");
    assert!(store.pieces(first).is_none());
    assert_eq!(
        released(&store),
        [(first.get(), GraphicsReleaseReason::StoreFull)]
    );
}

#[test]
fn a_unit_larger_than_the_cap_is_never_retained() {
    let (vt, keys) = drawn(1);
    let mut store = store(size() - 1);
    let first = keys.first().copied().expect("drawn");
    let notice = store
        .admit(unit(first, 1), &vt.images())
        .expect("over the cap");
    assert_eq!(notice.evicted_bytes.get(), size());
    assert_eq!(store.retained_bytes(), 0);
    assert!(store.pieces(first).is_none());
}

#[test]
fn a_series_growing_past_the_cap_is_evicted_whole() {
    let (vt, keys) = drawn(2);
    let shown = vt.images();
    let mut store = store(size());
    let first = keys.first().copied().expect("drawn");
    assert_eq!(store.admit(unit(first, 1), &shown), None);
    let mut run = unit(first, 1);
    run.bytes = Arc::from(b"\x1b_Gm=0;AA\x1b\\".as_slice());
    let notice = store.admit(run, &shown).expect("over the cap");
    assert_eq!(notice.evicted_sequences.get(), 1, "the series goes whole");
    assert_eq!(store.retained_bytes(), 0);
}

#[test]
fn later_runs_of_a_series_join_their_first_run() {
    let (vt, keys) = drawn(1);
    let mut store = store(1024);
    let key = keys.first().copied().expect("drawn");
    let mut run = unit(key, 1);
    drop(store.admit(run.clone(), &vt.images()));
    run.bytes = Arc::from(b"\x1b_Gm=0;BB\x1b\\".as_slice());
    drop(store.admit(run, &vt.images()));
    assert_eq!(store.pieces(key).map(<[_]>::len), Some(2));
    store.forget(key);
    assert!(store.pieces(key).is_none());
    assert_eq!(store.retained_bytes(), 0);
}

#[test]
fn a_sync_forgets_what_the_engine_released_with_its_reason() {
    let (mut vt, keys) = drawn(2);
    let mut store = store(1024);
    for (id, key) in (1..).zip(&keys) {
        drop(store.admit(unit(*key, id), &vt.images()));
    }
    drop(vt.feed("\r\n".repeat(6).as_bytes()));
    store.sync(&vt.images());
    let first = keys.first().copied().expect("drawn");
    assert_eq!(
        released(&store),
        [(first.get(), GraphicsReleaseReason::ScrolledOut)]
    );
    assert_eq!(store.retained_bytes(), size());
    store.sync(&ImageSet::default());
    assert_eq!(released(&store).len(), 2);
    assert_eq!(store.retained_bytes(), 0);
}

#[test]
fn an_image_released_before_its_bytes_arrive_is_still_reported_once() {
    let (mut vt, keys) = drawn(1);
    drop(vt.feed("\r\n".repeat(8).as_bytes()));
    let key = keys.first().copied().expect("drawn");
    let mut store = store(1024);
    for _ in 0..2 {
        assert_eq!(store.admit(unit(key, 1), &vt.images()), None);
    }
    assert!(store.pieces(key).is_none());
    assert_eq!(
        released(&store),
        [(key.get(), GraphicsReleaseReason::ScrolledOut)]
    );
    let stray = OutputOffset::new(999);
    drop(store.admit(unit(stray, 9), &vt.images()));
    assert_eq!(
        released(&store).len(),
        1,
        "a sequence the engine never retained is not listed"
    );
}

#[test]
fn releases_are_listed_in_receipt_order_however_reads_were_split() {
    let (mut vt, keys) = drawn(1);
    let mut store = store(1024);
    let first = keys.first().copied().expect("drawn");
    drop(store.admit(unit(first, 1), &vt.images()));
    let second = vt.offset();
    drop(vt.feed(&image(2)));
    drop(vt.feed("\r\n".repeat(8).as_bytes()));
    assert_eq!(store.admit(unit(second, 2), &vt.images()), None);
    store.sync(&vt.images());
    let order: Vec<u64> = released(&store).into_iter().map(|(key, _)| key).collect();
    assert_eq!(order, [first.get(), second.get()]);
}

#[test]
fn the_released_list_is_bounded() {
    let (vt, keys) = drawn(1);
    let key = keys.first().copied().expect("drawn");
    let mut store = store(1);
    for _ in 0..=RELEASED_LIMIT {
        drop(store.admit(unit(key, 1), &vt.images()));
    }
    let list = store.released();
    assert_eq!(list.len(), RELEASED_LIMIT);
    let expected =
        GraphicsReleased::new(ByteOffset::new(key.get()), GraphicsReleaseReason::StoreFull);
    assert!(list.iter().all(|item| *item == expected));
}
