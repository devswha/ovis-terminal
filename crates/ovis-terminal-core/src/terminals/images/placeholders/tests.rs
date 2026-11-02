//! Placeholders and the placement index name exactly the images the store holds (plan todo 85).

use std::sync::Arc;

use ovis_terminal_protocol::domain::graphics::{
    GraphicsCaps, GraphicsIndexEntry, GraphicsPlaceholder, GraphicsProtocol,
};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::ByteCount;
use ovis_terminal_vt::{Geometry, HistoryLimit, OutputOffset, Vt, VtConfig};

use crate::terminals::images::{ImageStore, ReleasedUnit};

const PAYLOAD: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB";

fn image(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=T,f=100,i={id},c=3,r=1;{PAYLOAD}\x1b\\").into_bytes()
}

/// An engine of `rows` rows that drew `count` images on successive rows, and their keys.
fn drawn(count: u32, rows: u16) -> (Vt, Vec<OutputOffset>) {
    let geometry = Geometry::new(20, rows).expect("valid geometry");
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

fn admitted(vt: &Vt, keys: &[OutputOffset], cap: u64) -> ImageStore {
    let mut store = store(cap);
    for (id, key) in (1..).zip(keys) {
        let unit = ReleasedUnit::new(*key, GraphicsProtocol::Kitty, Arc::from(image(id)));
        drop(store.admit(unit, &vt.images()));
    }
    store
}

fn size() -> u64 {
    u64::try_from(image(1).len()).expect("small")
}

/// (image id, row, bytes) of each placeholder.
fn summary(placeholders: &[GraphicsPlaceholder]) -> Vec<(u64, u16, u64)> {
    placeholders
        .iter()
        .map(|item| (item.image_id.get(), item.row, item.bytes.get()))
        .collect()
}

#[test]
fn placeholder_ids_are_the_store_keys_with_the_store_entry_sizes() {
    let (vt, keys) = drawn(2, 6);
    let store = admitted(&vt, &keys, 1024);
    let placeholders = store.placeholders(&vt.screen().images);
    let stored: Vec<(u64, u64)> = keys
        .iter()
        .filter_map(|key| {
            let bytes = store
                .pieces(*key)?
                .iter()
                .map(|piece| piece.len())
                .sum::<usize>();
            Some((key.get(), u64::try_from(bytes).expect("small")))
        })
        .collect();
    assert_eq!(stored.len(), 2);
    assert_eq!(
        summary(&placeholders),
        stored
            .iter()
            .zip(0..)
            .map(|(&(key, bytes), row)| (key, row, bytes))
            .collect::<Vec<_>>()
    );
    assert!(stored.iter().all(|&(_, bytes)| bytes == size()));
    assert_eq!(
        placeholders
            .iter()
            .map(|item| (item.col, item.rows, item.cols))
            .collect::<Vec<_>>(),
        vec![(0, 1, 3), (0, 1, 3)]
    );
}

#[test]
fn an_image_the_store_evicted_has_no_placeholder_while_the_screen_still_shows_it() {
    let (vt, keys) = drawn(2, 6);
    let store = admitted(&vt, &keys, size());
    assert_eq!(vt.screen().images.len(), 2);
    let (first, second) = (keys.first().copied(), keys.get(1).copied());
    assert_eq!(first.and_then(|key| store.pieces(key)), None);
    assert_eq!(
        summary(&store.placeholders(&vt.screen().images)),
        second
            .map(|key| (key.get(), 1, size()))
            .into_iter()
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_image_scrolled_off_and_synced_out_of_the_store_has_no_placeholder() {
    let (mut vt, keys) = drawn(1, 3);
    let mut store = admitted(&vt, &keys, 1024);
    assert_eq!(store.placeholders(&vt.screen().images).len(), 1);
    drop(vt.feed(b"\r\n\r\n\r\n"));
    store.sync(&vt.images());
    assert!(keys.iter().all(|key| store.pieces(*key).is_none()));
    assert_eq!(store.placeholders(&vt.screen().images), Vec::new());
}

#[test]
fn an_image_whose_bytes_never_reached_the_store_has_no_placeholder() {
    let (vt, keys) = drawn(1, 4);
    let mut store = admitted(&vt, &keys, 1024);
    for key in &keys {
        store.forget(*key);
    }
    assert_eq!(vt.screen().images.len(), 1);
    assert_eq!(store.placeholders(&vt.screen().images), Vec::new());
}

#[test]
fn a_placeholder_never_carries_the_payload() {
    let (vt, keys) = drawn(1, 4);
    let store = admitted(&vt, &keys, 1024);
    let encoded = serde_json::to_string(&store.placeholders(&vt.screen().images))
        .expect("placeholders encode");
    assert!(!encoded.contains(PAYLOAD), "{encoded}");
    assert!(encoded.len() < 200, "{encoded}");
}

/// (image id, buffer, row, col, rows, cols, bytes) of each index entry, as encoded.
fn indexed(entries: &[GraphicsIndexEntry]) -> serde_json::Value {
    serde_json::to_value(entries).expect("index encodes")
}

#[test]
fn the_index_lists_a_bare_transmission_and_an_image_on_the_inactive_screen() {
    let geometry = Geometry::new(20, 4).expect("valid geometry");
    let mut vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(10)));
    let transmission = b"\x1b_Ga=t,i=5;AAAA\x1b\\".to_vec();
    drop(vt.feed(&transmission));
    let shown_at = vt.offset();
    drop(vt.feed(&image(1)));
    drop(vt.feed(b"\x1b[?1049h"));
    let mut store = store(1024);
    for (key, bytes) in [(OutputOffset::new(0), transmission), (shown_at, image(1))] {
        let unit = ReleasedUnit::new(key, GraphicsProtocol::Kitty, Arc::from(bytes));
        drop(store.admit(unit, &vt.images()));
    }
    assert_eq!(store.placeholders(&vt.screen().images), Vec::new());
    assert_eq!(
        indexed(&store.index(&vt.images())),
        serde_json::json!([
            {"image_id": "0", "format": "kitty", "bytes": "17", "placement": null},
            {"image_id": shown_at.get().to_string(), "format": "kitty", "bytes": size().to_string(),
             "placement": {"buffer": "primary", "row": 0, "col": 0, "rows": 1, "cols": 3}}
        ])
    );
}

#[test]
fn the_index_omits_what_the_store_evicted() {
    let (vt, keys) = drawn(2, 6);
    let store = admitted(&vt, &keys, size());
    let ids: Vec<u64> = store
        .index(&vt.images())
        .iter()
        .map(|entry| entry.image_id.get())
        .collect();
    assert_eq!(
        ids,
        keys.iter().skip(1).map(|key| key.get()).collect::<Vec<_>>()
    );
    assert!(
        !indexed(&store.index(&vt.images()))
            .to_string()
            .contains(PAYLOAD)
    );
}
