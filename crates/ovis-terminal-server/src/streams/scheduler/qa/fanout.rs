use std::sync::Arc;

use ovis_terminal_vt::OutputOffset;
use tokio::sync::broadcast::error::RecvError;

use super::super::OutputEvent;
use super::{await_event, parsed_through, screenful, setup, spawn};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_viewer_receives_the_same_shared_chunk() {
    let (handle, feed, task) = spawn(&setup());
    let mut viewers: Vec<_> = (0..8).map(|_| handle.subscribe_output()).collect();
    let chunk = screenful(7);
    feed.send(&chunk).await.expect("the actor takes output");

    let mut received = Vec::new();
    for viewer in &mut viewers {
        let bytes = await_event(viewer, |event| match event {
            OutputEvent::Chunk { offset, bytes } if *offset == OutputOffset::ZERO => {
                Some(Arc::clone(bytes))
            }
            _ => None,
        })
        .await;
        received.push(bytes);
    }
    let first = received.first().expect("eight viewers");
    assert_eq!(&first[..], &chunk[..]);
    assert!(
        received.iter().all(|bytes| Arc::ptr_eq(bytes, first)),
        "eight viewers hold one allocation, not eight copies"
    );
    let end = OutputOffset::new(u64::try_from(chunk.len()).expect("fits"));
    for viewer in &mut viewers {
        await_event(viewer, |event| match event {
            OutputEvent::Parsed { offset } if *offset == end => Some(()),
            _ => None,
        })
        .await;
    }

    handle.stop().expect("stop queued");
    let report = task.join().await.expect("joined");
    assert_eq!(report.parsed_bytes, end.get());
}

/// One output event per accepted chunk and one per parse turn: with chunks of exactly one
/// parse budget, `chunks` chunks publish `2 * chunks` events, so a subscriber that never
/// reads loses exactly the events beyond the retained window, and is told so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_subscriber_is_told_exactly_how_many_events_it_lost() {
    const CHUNKS: u64 = 100;
    const RETAINED: u64 = 64;
    let chunk = 16 * 1024;
    let (handle, feed, task) = spawn(&setup());
    let mut slow = handle.subscribe_output();
    let end = OutputOffset::new(CHUNKS.saturating_mul(u64::try_from(chunk).expect("fits")));
    let fast = tokio::spawn(parsed_through(handle.subscribe_output(), end));
    for seed in 0..CHUNKS {
        let fill = b'a'.saturating_add(u8::try_from(seed % 8).expect("fits"));
        feed.send(&vec![fill; chunk])
            .await
            .expect("the actor takes output");
    }
    fast.await.expect("the fast viewer saw the end");

    let lost = match slow.recv().await {
        Err(RecvError::Lagged(missed)) => missed,
        other => panic!("expected a loss count, got {other:?}"),
    };
    assert_eq!(
        lost,
        2 * CHUNKS - RETAINED,
        "every event beyond the window is counted"
    );
    let mut kept = 0_u64;
    while let Ok(_event) = slow.try_recv() {
        kept = kept.saturating_add(1);
    }
    assert_eq!(kept, RETAINED, "the newest window is still delivered");
    handle.stop().expect("stop queued");
    let report = task.join().await.expect("joined");
    assert_eq!(report.parse_turns, CHUNKS);
}
