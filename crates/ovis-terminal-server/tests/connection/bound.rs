//! The per-connection unwritten-byte bound against a real peer that stops reading.

use std::sync::Arc;

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_server::conn::{Call, RequestHandler};
use serde_json::{Value, json};
use tokio::sync::watch;

use super::support::{Fixture, TestResult, WAIT, at, request};

const RECORD: usize = 8 * 1024 * 1024;
const BOUND: u64 = 64 * 1024 * 1024;

/// Queues 8 MiB notifications until the connection refuses one.
struct Flood {
    refused: watch::Sender<Option<(u64, &'static str)>>,
}

impl RequestHandler for Flood {
    fn methods(&self) -> Vec<&'static str> {
        vec!["doc.get", "events.subscribe"]
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        if call.request.method() == "doc.get" {
            return Ok(json!({ "name": "b" }));
        }
        let mut accepted = 0_u64;
        loop {
            let event = json!({ "bytes": "x".repeat(RECORD) });
            match call.notifier.notify("event", event).await {
                Ok(()) => accepted = accepted.saturating_add(1),
                Err(error) => {
                    self.refused.send_replace(Some((accepted, error.kind())));
                    return Ok(json!({}));
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_that_stops_reading_is_closed_at_the_unwritten_byte_bound() -> TestResult {
    let fixture = Fixture::new()?;
    let (refused, mut outcome) = watch::channel(None);
    let server = fixture.serve(1, Arc::new(Flood { refused })).await?;
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
    let refused = *tokio::time::timeout(WAIT, outcome.wait_for(Option::is_some)).await??;
    let (accepted, kind) = refused.ok_or("an outcome")?;
    assert_eq!(kind, "backpressure");
    let ceiling = BOUND / u64::try_from(RECORD)?;
    assert!(
        accepted < ceiling,
        "{accepted} records of 8 MiB fit under 64 MiB"
    );

    other
        .send(&request("b", "doc.get", &json!({ "name": "b" })))
        .await?;
    assert_eq!(at(&other.expect_next().await?, "/result/name"), "b");

    let mut seen = Vec::new();
    while let Some(message) = stalled.next().await? {
        seen.push(
            at(&message, "/method")
                .as_str()
                .unwrap_or("reply")
                .to_owned(),
        );
        if at(&message, "/method") == "$/closing" {
            assert_eq!(at(&message, "/params/reason"), "backpressure");
            assert_eq!(at(&message, "/params/limit_bytes"), "67108864");
            let queued: u64 = at(&message, "/params/queued_bytes")
                .as_str()
                .ok_or("string")?
                .parse()?;
            assert!(queued <= BOUND, "queued {queued} stays within the bound");
        }
    }
    assert_eq!(
        seen.last().map(String::as_str),
        Some("$/closing"),
        "{seen:?}"
    );
    let events = seen.iter().filter(|method| *method == "event").count();
    assert!(
        u64::try_from(events)? < accepted,
        "queued records were discarded: {seen:?}"
    );

    drop(other);
    let report = server.shutdown().await?;
    assert_eq!(report.failures().get("backpressure"), Some(&1));
    assert_eq!(report.aborted(), 0);
    Ok(())
}
