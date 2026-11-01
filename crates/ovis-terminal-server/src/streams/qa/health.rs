//! A real terminal stream hub reporting to health.
//!
//! A viewer that never acknowledges is resynchronized, and its backlog and replaced byte
//! ranges are what `server.metrics` reports; a viewer that keeps up reports nothing.

use std::sync::Arc;

use ovis_terminal_authz::Permission;
use ovis_terminal_core::health::{Health, Observed, RecoveryLedger};
use ovis_terminal_core::recovery::Incidents;
use ovis_terminal_protocol::domain::health::{ConsumerRef, LossRange};
use ovis_terminal_protocol::schema::ids;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::methods::server::{ConsumerQueue, ServerMetricsResult};
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};
use ovis_terminal_server::streams::StreamLimits;

use super::TestResult;
use super::fixture::{Fixture, TERMINAL};
use super::socket::runtime;
use super::viewer::{Viewer, open_params};

const OBSERVER: &[Permission] = &[Permission::Observe];
const BACKLOG: u64 = 64 * 1024;

fn health(dir: &std::path::Path) -> TestResult<Arc<Health>> {
    let identity = runtime(1)?;
    let boot = ids::ServerBootId::new(identity.server_boot_id().to_string());
    let ledger = RecoveryLedger::open(Incidents::new(dir), (boot, TimestampMs::new(1)), None)?;
    Ok(Arc::new(Health::new(identity, ledger)))
}

fn metrics(health: &Health) -> TestResult<ServerMetricsResult> {
    Ok(health.metrics(&Observed::new(EventSeq::new(0), Vec::new()))?)
}

/// The row of the one stream `client` attached, which must be a stream of the terminal.
fn row<'a>(read: &'a ServerMetricsResult, client: &str) -> TestResult<&'a ConsumerQueue> {
    let row = read
        .consumers
        .iter()
        .find(|row| row.client_id.as_str() == client)
        .ok_or("the client's stream has no metrics row")?;
    match &row.consumer {
        ConsumerRef::Stream { terminal_id, .. } if terminal_id.as_str() == TERMINAL => Ok(row),
        other => Err(format!("{client} reports {other:?}, not a stream of {TERMINAL}").into()),
    }
}

pub(crate) async fn a_real_stalled_viewer_reports_its_backlog_and_resynced_ranges() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ovis-stream-health-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir)?;
    let health = health(&dir)?;
    let limits = StreamLimits::advertised(&Limits::v1()).with_backlog_bytes(BACKLOG);
    let mut fx = Fixture::reporting(limits, 64 * 1024 * 1024, Arc::clone(&health)).await?;
    fx.issue("healthy", OBSERVER)?;
    fx.issue("zero", OBSERVER)?;
    fx.publish(b"$ ")?;
    // Given a viewer that reads but never acknowledges and one that keeps up.
    let zero = Viewer::open(
        fx.connect("zero").await?,
        &open_params("s-1", "zero", "observe"),
        false,
    )
    .await?;
    let mut healthy = Viewer::open(
        fx.connect("healthy").await?,
        &open_params("s-2", "healthy", "observe"),
        true,
    )
    .await?;
    healthy.until(|viewer| !viewer.received.is_empty()).await?;
    let mut expected = healthy.received.len();
    // When output floods far past the backlog bound, paced only by the healthy reader.
    let line = b"0123456789abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRST\r\n";
    let flood: Vec<u8> = line.iter().copied().cycle().take(600 * 1024).collect();
    for chunk in flood.chunks(16 * 1024) {
        fx.publish(chunk)?;
        expected = expected.saturating_add(chunk.len());
        healthy
            .until(|viewer| viewer.received.len() >= expected)
            .await?;
    }
    // Then both streams are counted under the clients that attached them.
    let read = metrics(&health)?;
    assert_eq!((read.streams.get(), read.subscriptions.get()), (2, 0));
    let stalled = row(&read, "zero")?;
    // And the stalled viewer reports an unsent backlog and only resynchronized byte
    // ranges, each within the output it never received.
    assert!(stalled.queue_bytes.get() > 0, "{stalled:?}");
    assert!(!stalled.losses.is_empty(), "{stalled:?}");
    let raw = u64::try_from(fx.raw.len())?;
    for loss in &stalled.losses {
        assert!(loss.resynced, "{loss:?}");
        match loss.range {
            LossRange::Bytes {
                first_offset,
                last_offset,
            } => assert!(first_offset.get() <= last_offset.get() && last_offset.get() < raw),
            LossRange::Events { .. } => return Err("a stream lost an event range".into()),
            _ => return Err("unknown loss range".into()),
        }
    }
    // And the viewer that kept up has nothing queued and lost nothing.
    let kept_up = row(&read, "healthy")?;
    assert_eq!(kept_up.queue_bytes.get(), 0, "{kept_up:?}");
    assert!(kept_up.losses.is_empty(), "{kept_up:?}");
    drop((zero, healthy));
    fx.close().await?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
