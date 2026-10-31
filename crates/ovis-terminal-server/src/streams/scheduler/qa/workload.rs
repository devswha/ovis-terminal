//! The profile's workload: paced program output, parsed-offset trackers, the drain before
//! stop, and the clients' control-event watchers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinSet;

use super::super::{OutputEvent, OutputFeed, TerminalEvent};
use super::screenful;

const TICK: Duration = Duration::from_millis(16);
const PER_TICK: usize = 1024;
/// Hang guard for parsing the accepted output on a slow or contended host.
const DRAIN_GUARD: Duration = Duration::from_secs(300);

/// Commit-to-delivery latency of every committed control event one client sees.
pub(super) async fn watch_events(
    mut events: tokio::sync::broadcast::Receiver<TerminalEvent>,
) -> (Vec<Duration>, u64) {
    let mut latency = Vec::new();
    let mut lost = 0_u64;
    loop {
        match events.recv().await {
            Ok(
                TerminalEvent::Admitted { at, .. }
                | TerminalEvent::Withdrawn { at, .. }
                | TerminalEvent::Resized { at, .. },
            ) => latency.push(at.elapsed()),
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => lost = lost.saturating_add(missed),
            Err(RecvError::Closed) => return (latency, lost),
        }
    }
}

/// The newest parsed offset of one terminal, kept current by a lag-tolerant reader: the
/// final `Parsed` event is the newest one published, so lag never hides it.
pub(super) fn track_parsed(mut output: broadcast::Receiver<OutputEvent>) -> watch::Receiver<u64> {
    let (parsed, tracked) = watch::channel(0_u64);
    tokio::spawn(async move {
        loop {
            match output.recv().await {
                Ok(OutputEvent::Parsed { offset }) => {
                    parsed.send_replace(offset.get());
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
        }
    });
    tracked
}

/// 64 KiB/s of dense output per terminal for as long as the clients run; returns the
/// terminal and the bytes its feed accepted.
pub(super) async fn produce(
    terminal: usize,
    feed: OutputFeed,
    (done, total): (watch::Receiver<bool>, Arc<AtomicU64>),
) -> (usize, u64) {
    let stream: Vec<u8> = (0..64)
        .flat_map(|seed| screenful(terminal.wrapping_mul(64).wrapping_add(seed)))
        .collect();
    let len = u64::try_from(PER_TICK).expect("fits");
    let mut pace = tokio::time::interval(TICK);
    let mut sent = 0_u64;
    let mut at = 0_usize;
    while !*done.borrow() {
        pace.tick().await;
        if stream.get(at..at.saturating_add(PER_TICK)).is_none() {
            at = 0;
        }
        let next = at.saturating_add(PER_TICK);
        let piece = stream.get(at..next).expect("the stream holds one tick");
        if feed.send(piece).await.is_err() {
            break;
        }
        at = next;
        sent = sent.saturating_add(len);
        total.fetch_add(len, Ordering::AcqRel);
    }
    (terminal, sent)
}

/// Joins the producers, then waits until each terminal parsed everything its feed
/// accepted: stop is served ahead of queued parse work, so stopping earlier would leave
/// output unparsed on a slow host. The timeout only guards against a hang.
pub(super) async fn drain(
    mut producers: JoinSet<(usize, u64)>,
    mut trackers: Vec<watch::Receiver<u64>>,
) -> u64 {
    let mut output_bytes = 0_u64;
    while let Some(joined) = producers.join_next().await {
        let (terminal, sent) = joined.expect("producer joined");
        output_bytes = output_bytes.saturating_add(sent);
        let tracker = trackers
            .get_mut(terminal)
            .expect("one tracker per terminal");
        tokio::time::timeout(DRAIN_GUARD, tracker.wait_for(|parsed| *parsed >= sent))
            .await
            .expect("the terminal parses its accepted output")
            .expect("the tracker runs");
    }
    output_bytes
}
