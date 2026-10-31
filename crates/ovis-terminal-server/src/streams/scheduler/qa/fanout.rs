use std::sync::Arc;

use ovis_terminal_vt::OutputOffset;

use super::super::{OutputEvent, spawn_terminal};
use super::{await_event, screenful, setup, vt};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_viewer_receives_the_same_shared_chunk() {
    let (handle, feed, task) = spawn_terminal(vt(), tokio::io::sink(), &setup());
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
