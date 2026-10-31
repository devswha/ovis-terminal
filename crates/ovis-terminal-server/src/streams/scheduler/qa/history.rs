//! The actor records accepted output in its disk history and serves reads by offset from
//! the history thread; retention advances a visible floor.

use ovis_terminal_vt::OutputOffset;

use super::super::{Engine, HistoryConfig, HistoryStore, SchedulerError, spawn_terminal};
use super::{parsed_through, setup, spawn, vt};

const KIB: usize = 1024;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_output_is_readable_from_disk_until_retention_evicts_it() {
    let dir = std::env::temp_dir().join(format!("ovt56a-{}", uuid::Uuid::now_v7()));
    let config = HistoryConfig::new(4 * 1024, 16 * 1024).expect("valid");
    let store = HistoryStore::open(&dir, config).expect("opens");
    let engine = Engine::new(vt()).with_history(store);
    let (handle, feed, task) = spawn_terminal(engine, tokio::io::sink(), &setup()).expect("spawns");
    let bytes: Vec<u8> = b"0123456789abcdef"
        .iter()
        .copied()
        .cycle()
        .take(64 * KIB)
        .collect();
    let end = OutputOffset::new(u64::try_from(bytes.len()).expect("fits"));
    let parsed = tokio::spawn(parsed_through(handle.subscribe_output(), end));
    feed.send(&bytes).await.expect("the actor takes output");
    parsed.await.expect("joined");

    let tail = handle
        .read_history(OutputOffset::new(56 * 1024), 8 * KIB)
        .await
        .expect("the newest bytes are on disk");
    assert_eq!(Some(tail.bytes.as_slice()), bytes.get(56 * KIB..));
    assert_eq!(tail.end, end);
    let floor = tail.floor;
    assert!(floor > OutputOffset::ZERO, "retention advanced the floor");

    let evicted = handle
        .read_history(OutputOffset::ZERO, 16)
        .await
        .expect_err("the oldest bytes were evicted");
    assert_eq!(evicted.kind(), "history_gap");
    let from_floor = handle
        .read_history(floor, 4)
        .await
        .expect("the floor is readable");
    let start = usize::try_from(floor.get()).expect("fits");
    assert_eq!(
        Some(from_floor.bytes.as_slice()),
        bytes.get(start..start.saturating_add(4))
    );

    handle.stop().expect("stop queued");
    let report = task.join().await.expect("joined");
    let stats = report.history.expect("the history thread was joined");
    assert_eq!((stats.end, stats.floor), (end.get(), floor.get()));
    assert_eq!(
        stats.disk_bytes,
        stats.end.saturating_sub(stats.floor),
        "measured from the files"
    );
    assert!(stats.evicted_segments > 0);
    assert_eq!(stats.fault, None);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_terminal_without_history_refuses_history_reads() {
    let (handle, _feed, task) = spawn(&setup());
    let refused = handle
        .read_history(OutputOffset::ZERO, 1)
        .await
        .expect_err("no history");
    assert!(matches!(refused, SchedulerError::NoHistory));
    assert_eq!(refused.kind(), "invalid_params");
    handle.stop().expect("stop queued");
    assert_eq!(task.join().await.expect("joined").history, None);
}
