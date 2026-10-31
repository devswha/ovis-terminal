//! The per-connection unwritten-byte bound against real peers.

use std::sync::Arc;

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_server::conn::{Call, RequestHandler};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use super::support::{Fixture, TestResult, WAIT, at, request};

const RECORD: usize = 8 * 1024 * 1024;
const BOUND: usize = 64 * 1024 * 1024;

/// `events.subscribe` queues one 8 MiB notification, waits for the test, then queues more
/// until one is refused; `doc.set` queues a single notification larger than the bound.
struct Flood {
    refused: watch::Sender<Option<(u64, &'static str)>>,
    /// Released by the test once the first record is blocked mid-write.
    flood: Arc<Notify>,
}

impl Flood {
    async fn push(&self, call: &Call, size: usize) -> bool {
        let event = json!({ "bytes": "x".repeat(size) });
        match call.notifier.notify("event", event).await {
            Ok(()) => true,
            Err(error) => {
                let accepted = self.refused.borrow().map_or(0, |(count, _)| count);
                self.refused.send_replace(Some((accepted, error.kind())));
                false
            }
        }
    }
}

impl RequestHandler for Flood {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get", "doc.set", "events.subscribe"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        match call.request.method() {
            "doc.get" => return Ok(json!({ "name": "b" })),
            "doc.set" => {
                self.push(&call, BOUND.saturating_add(1)).await;
            }
            _ => {
                let mut accepted = u64::from(self.push(&call, RECORD).await);
                self.flood.notified().await;
                while self.push(&call, RECORD).await {
                    accepted = accepted.saturating_add(1);
                }
                let kind = self.refused.borrow().map_or("none", |(_, kind)| kind);
                self.refused.send_replace(Some((accepted, kind)));
            }
        }
        Ok(json!({}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_that_never_reads_is_closed_and_released_without_shutdown() -> TestResult {
    let fixture = Fixture::new()?;
    let (refused, mut outcome) = watch::channel(None);
    let flood = Arc::new(Notify::new());
    let handler = Flood {
        refused,
        flood: Arc::clone(&flood),
    };
    let server = fixture.serve(1, Arc::new(handler)).await?;
    let mut progress = server.progress();
    let mut stalled = fixture.connect().await?;
    let hello = stalled.hello_framed("1", &fixture.bearer).await?;
    assert_eq!(
        at(&hello, "/result/limits/connection_unwritten_bytes"),
        "67108864"
    );
    let mut other = fixture.connect().await?;
    other.hello("1", &fixture.bearer).await?;

    stalled
        .send(&request("flood", "events.subscribe", &json!({})))
        .await?;
    // Bytes of the first 8 MiB record have arrived unread, so its write is blocked midway.
    stalled.readable().await?;
    flood.notify_one();
    let refused = *tokio::time::timeout(WAIT, outcome.wait_for(Option::is_some)).await??;
    let (accepted, kind) = refused.ok_or("an outcome")?;
    assert_eq!(kind, "backpressure");
    assert!(
        accepted < u64::try_from(BOUND / RECORD)?,
        "{accepted} records fit"
    );

    // The stalled peer is never read from. The accept loop reports its connection as
    // joined, which means its writer, queue and read half are gone and the socket is closed.
    let closed = |report: &ovis_terminal_server::listen::ServeReport| {
        report.failures().get("backpressure") == Some(&1)
    };
    tokio::time::timeout(WAIT, progress.wait_for(closed)).await??;

    other
        .send(&request("b", "doc.get", &json!({ "name": "b" })))
        .await?;
    assert_eq!(at(&other.expect_next().await?, "/result/name"), "b");
    drop((other, stalled));
    let report = server.shutdown().await?;
    assert_eq!(
        report.failures().get("backpressure"),
        Some(&1),
        "{report:?}"
    );
    assert_eq!(report.aborted(), 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_overflow_at_a_record_boundary_sends_a_readable_closing() -> TestResult {
    let fixture = Fixture::new()?;
    let (refused, mut outcome) = watch::channel(None);
    let flood = Arc::new(Notify::new());
    let server = fixture.serve(1, Arc::new(Flood { refused, flood })).await?;
    let mut client = fixture.connect().await?;
    client.hello_framed("1", &fixture.bearer).await?;
    client
        .send(&request("big", "doc.set", &json!({ "name": "x" })))
        .await?;
    tokio::time::timeout(WAIT, outcome.wait_for(Option::is_some)).await??;

    let closing = client.expect_next().await?;
    assert_eq!(at(&closing, "/method"), "$/closing");
    assert_eq!(at(&closing, "/params/reason"), "backpressure");
    assert_eq!(at(&closing, "/params/limit_bytes"), "67108864");
    assert_eq!(at(&closing, "/params/queued_bytes"), "0");
    assert!(
        client.next().await?.is_none(),
        "the socket closes after $/closing"
    );
    let report = server.shutdown().await?;
    assert_eq!(
        report.failures().get("backpressure"),
        Some(&1),
        "{report:?}"
    );
    assert_eq!(report.aborted(), 0);
    Ok(())
}
