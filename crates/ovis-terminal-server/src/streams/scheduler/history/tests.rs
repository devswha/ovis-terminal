use std::path::{Path, PathBuf};

use ovis_terminal_vt::OutputOffset;
use proptest::prelude::*;

use super::{HistoryConfig, HistoryError, HistoryStore};

/// A private directory removed when the test ends.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("ovt56h-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("create the test directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn files(&self) -> usize {
        std::fs::read_dir(&self.0).expect("list").count()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn config(segment: u64, retained: u64) -> HistoryConfig {
    HistoryConfig::new(segment, retained).expect("valid config")
}

fn stream(len: usize) -> Vec<u8> {
    b"abcdefghijklmnopqrstuvwxyz"
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

fn part(bytes: &[u8], range: std::ops::Range<usize>) -> &[u8] {
    bytes.get(range).expect("inside the stream")
}

#[test]
fn retention_evicts_the_oldest_segments_and_advances_the_floor() {
    let dir = Dir::new();
    let mut store = HistoryStore::open(dir.path(), config(4, 10)).expect("opens");
    let bytes = stream(30);
    for chunk in bytes.chunks(3) {
        store.append(chunk).expect("appends");
    }
    let stats = store.stats().expect("stats");
    // 30 bytes in 4-byte segments; keeping more than 10 bytes needs at most 3 segments.
    assert_eq!((stats.end, stats.floor), (30, 20));
    assert_eq!((stats.segments, stats.evicted_segments), (3, 5));
    assert_eq!(stats.disk_bytes, 10, "measured from the files");
    assert_eq!(dir.files(), 3, "evicted segments are deleted");

    let below = store.read(OutputOffset::new(19), 4).expect_err("evicted");
    assert!(matches!(
        below,
        HistoryError::Evicted { floor, .. } if floor == OutputOffset::new(20)
    ));
    assert_eq!(below.kind(), "history_gap");
    let read = store
        .read(OutputOffset::new(20), 100)
        .expect("reads from the floor");
    assert_eq!(read.bytes, part(&bytes, 20..30));
    assert_eq!(read.floor, OutputOffset::new(20));
    let beyond = store
        .read(OutputOffset::new(31), 1)
        .expect_err("past the end");
    assert_eq!(beyond.kind(), "invalid_params");
    let at_end = store
        .read(OutputOffset::new(30), 8)
        .expect("the end is readable");
    assert_eq!(at_end.bytes, Vec::<u8>::new());
}

#[test]
fn reopening_rebuilds_the_index_and_continues_the_stream() {
    let dir = Dir::new();
    let bytes = stream(23);
    let mut store = HistoryStore::open(dir.path(), config(5, 100)).expect("opens");
    store.append(part(&bytes, 0..13)).expect("appends");
    let before = store.stats().expect("stats");
    drop(store);

    let mut store = HistoryStore::open(dir.path(), config(5, 100)).expect("reopens");
    assert_eq!(store.stats().expect("stats"), before);
    store
        .append(part(&bytes, 13..23))
        .expect("appends after the recorded end");
    let read = store.read(OutputOffset::ZERO, 100).expect("reads");
    assert_eq!(read.bytes, bytes);
    assert_eq!(store.stats().expect("stats").segments, 5);
}

#[test]
fn a_gap_between_segment_files_is_refused_on_open() {
    let dir = Dir::new();
    std::fs::write(dir.path().join(format!("{:020}.seg", 0)), b"abc").expect("write");
    std::fs::write(dir.path().join(format!("{:020}.seg", 5)), b"fgh").expect("write");
    let refused = HistoryStore::open(dir.path(), config(4, 16)).expect_err("discontinuous");
    assert!(matches!(
        refused,
        HistoryError::Discontinuous { start, expected }
            if start == OutputOffset::new(5) && expected == OutputOffset::new(3)
    ));
    assert_eq!(refused.kind(), "history_unavailable");
}

#[test]
fn a_segment_larger_than_the_bound_is_an_invalid_config() {
    for (segment, retained) in [(0, 8), (9, 8)] {
        let refused = HistoryConfig::new(segment, retained).expect_err("invalid");
        assert_eq!(refused.kind(), "invalid_params");
    }
}

#[cfg(unix)]
#[test]
fn a_failed_append_stops_recording_with_a_typed_fault() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = Dir::new();
    let mut store = HistoryStore::open(dir.path(), config(4, 64)).expect("opens");
    store.append(b"abcd").expect("fills the first segment");
    let read_only = std::fs::Permissions::from_mode(0o500);
    std::fs::set_permissions(dir.path(), read_only).expect("chmod");
    let failed = store.append(b"e");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
    assert_eq!(
        failed.map_err(|error| error.kind()).err(),
        Some("history_unavailable")
    );
    let refused = store.append(b"f").expect_err("recording stopped");
    assert!(matches!(refused, HistoryError::Faulted(_)));
    assert!(store.stats().expect("stats").fault.is_some());
    assert_eq!(
        store.read(OutputOffset::ZERO, 8).expect("reads").bytes,
        b"abcd"
    );
}

proptest! {
    /// Any chunking and segment size: every read inside the retained window returns
    /// exactly the appended bytes, and the window never exceeds bound plus one segment.
    #[test]
    fn reads_inside_the_window_return_the_appended_bytes(
        chunks in prop::collection::vec(1_usize..40, 1..30),
        segment in 1_u64..16,
        extra in 0_u64..48,
        at in 0_u64..2_000,
        len in 0_usize..200,
    ) {
        let dir = Dir::new();
        let retained = segment.saturating_add(extra);
        let mut store = HistoryStore::open(dir.path(), config(segment, retained)).expect("opens");
        let bytes = stream(chunks.iter().sum());
        let mut from = 0_usize;
        for size in chunks {
            let to = from.saturating_add(size);
            store.append(part(&bytes, from..to)).expect("appends");
            from = to;
        }
        let stats = store.stats().expect("stats");
        let window = stats.end.saturating_sub(stats.floor);
        prop_assert_eq!(stats.end, u64::try_from(bytes.len()).expect("fits"));
        prop_assert!(window <= retained.saturating_add(segment));
        prop_assert_eq!(stats.disk_bytes, window);
        let offset = stats.floor.saturating_add(at.checked_rem(window.saturating_add(1)).unwrap_or(0));
        let read = store.read(OutputOffset::new(offset), len).expect("inside the window");
        let start = usize::try_from(offset).expect("fits");
        let stop = start.saturating_add(len).min(bytes.len());
        prop_assert_eq!(read.bytes, part(&bytes, start..stop).to_vec());
    }
}
