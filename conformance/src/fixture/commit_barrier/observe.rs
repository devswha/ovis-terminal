//! What the paged snapshot and the installed subscription each saw of the race.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ovis_terminal_core::commit::{Pager, Window};
use ovis_terminal_core::subscriptions::{Pulled, Started};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::domain::{Ballast, Barrier, Captured, HISTORY_BYTES};
use crate::error::RunnerError;
use crate::fixture::serve::at;

/// Bytes the probe asks for per page; the server's 1 MiB page bound cuts each reply.
const PAGE_ASK: usize = 8 << 20;
const BOUND: Duration = Duration::from_secs(20);

pub(super) fn now() -> Result<TimestampMs, RunnerError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(at("read the clock"))?;
    let millis = u64::try_from(elapsed.as_millis()).map_err(at("represent the clock"))?;
    Ok(TimestampMs::new(millis))
}

/// Reads every page of `window`, then asks once at its expiry.
pub(super) fn pages(
    pager: &Pager<Barrier, Ballast>,
    window: &Window,
) -> Result<Value, RunnerError> {
    let (mut bytes, mut offset) = (Vec::new(), Some(ByteOffset::new(0)));
    let (mut count, mut truncated, mut pinned) = (0_u64, 0_u64, true);
    let read_at = now()?;
    while let Some(start) = offset {
        let page = pager
            .page(&window.token, (start, PAGE_ASK, read_at))
            .map_err(at("read a snapshot page"))?;
        pinned &= page.cursor == window.cursor && page.total == window.total;
        count = count.saturating_add(1);
        truncated = truncated.saturating_add(u64::from(page.truncated));
        bytes.extend_from_slice(&page.data);
        offset = page.continuation;
    }
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    let captured: Captured = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("sections").cloned())
        .and_then(|sections| serde_json::from_value(sections).ok())
        .ok_or_else(|| at("decode the reassembled snapshot")("not a snapshot"))?;
    let revision = window.cursor.seq.get();
    let expired = pager
        .page(&window.token, (ByteOffset::new(0), 1, window.expires_ms))
        .err()
        .map_or("served", |error| error.kind());
    Ok(json!({
        "revision": revision.to_string(),
        "total_bytes": bytes.len().to_string(),
        "at_least_64_mib": bytes.len() >= HISTORY_BYTES && captured.history.len() == HISTORY_BYTES,
        "pages": count.to_string(),
        "truncated_pages": truncated.to_string(),
        "every_page_pinned": pinned,
        "digest_matches": digest == window.digest.as_str(),
        "holds_exactly_up_to_revision": captured.seqs == (1..=revision).collect::<Vec<_>>(),
        "expired": expired,
    }))
}

/// Pulls until `total` is delivered and checks snapshot plus deliveries cover 1..=total;
/// each delivered record's own origin must carry exactly its source's upstream position.
pub(super) fn subscription(
    installed: &Started<Barrier, Ballast>,
    total: u64,
) -> Result<Value, RunnerError> {
    let boundary = installed.snapshot_seq.get();
    let held = installed
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.sections.seqs == (1..=boundary).collect::<Vec<_>>());
    let deadline = Instant::now()
        .checked_add(BOUND)
        .ok_or_else(|| at("bound the drain")("clock overflow"))?;
    let (mut delivered, mut remote, mut origins_intact) = (Vec::new(), 0_u64, true);
    while delivered.last().copied().unwrap_or(boundary) < total && Instant::now() < deadline {
        match installed
            .feed
            .next(deadline)
            .map_err(at("pull the subscription"))?
        {
            Pulled::Events(batch) => {
                for event in &batch.events {
                    delivered.push(event.cursor.seq.get());
                    let upstream = event.origin.as_ref().map(|origin| origin.seq.get());
                    origins_intact &= match &event.data {
                        Barrier::Wrote { source, n } if source == "remote" => {
                            remote = remote.saturating_add(1);
                            upstream == Some(n.saturating_add(super::UPSTREAM_BASE))
                        }
                        _ => upstream.is_none(),
                    };
                }
            }
            Pulled::Heartbeat(_) => {}
            _ => return Err(at("pull the subscription")("delivery stopped")),
        }
    }
    let after: Vec<u64> = (boundary.saturating_add(1)..=total).collect();
    Ok(json!({
        "snapshot_seq": boundary.to_string(),
        "delivered": delivered.len().to_string(),
        "partitions_committed": held && delivered == after,
        "delivered_remote": remote.to_string(),
        "delivered_origins_intact": origins_intact,
    }))
}
