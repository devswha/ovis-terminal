//! Overflow and stall while a healthy viewer keeps receiving.
//!
//! An over-backlog viewer gets a gap plus checkpoint, a viewer without progress gets
//! CLOSE `stream_stalled`, and a connection that stopped reading loses its lease.

use std::time::Duration;

use ovis_terminal_authz::Permission;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::StreamLimits;

use super::TestResult;
use super::fixture::Fixture;
use super::socket::at;
use super::viewer::{Viewer, open_params};

const OBSERVER: &[Permission] = &[Permission::Observe];
const INTERACTOR: &[Permission] = &[Permission::Observe, Permission::Interact];
/// Stall bound of this scenario; the stall itself is the behavior under test.
const STALL: Duration = Duration::from_secs(2);
const BACKLOG: u64 = 64 * 1024;

pub(crate) async fn stalled_viewers_close_without_pausing_others() -> TestResult {
    let limits = StreamLimits::advertised(&Limits::v1())
        .with_backlog_bytes(BACKLOG)
        .with_stall_after(STALL);
    let mut fx = Fixture::new(limits, 64 * 1024 * 1024).await?;
    fx.issue("healthy", OBSERVER)?;
    fx.issue("zero", OBSERVER)?;
    fx.issue("hung", INTERACTOR)?;
    fx.publish(b"$ ")?;
    // Given a viewer that reads but never acknowledges, and an interact holder whose
    // connection never reads after attach.
    let mut zero = Viewer::open(
        fx.connect("zero").await?,
        &open_params("s-1", "zero", "observe"),
        false,
    )
    .await?;
    let hung = Viewer::open(
        fx.connect("hung").await?,
        &open_params("s-2", "hung", "interact"),
        false,
    )
    .await?;
    assert_eq!(at(&hung.result, "/result/outcome"), "ready");
    let mut healthy = Viewer::open(
        fx.connect("healthy").await?,
        &open_params("s-3", "healthy", "observe"),
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
    // Then the healthy viewer received everything, in order and without a gap.
    assert_eq!(healthy.received.len(), expected);
    assert_eq!(healthy.kinds("gap").len(), 0);
    assert_eq!(healthy.kinds("close").len(), 0);
    // And the non-acknowledging viewer gets a gap and a checkpoint, then CLOSE stalled.
    zero.until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let gap = zero.first("gap")?;
    let from: u64 = at(&gap, "/params/skipped/from_output_offset")
        .as_str()
        .unwrap_or("x")
        .parse()?;
    let to: u64 = at(&gap, "/params/skipped/to_output_offset")
        .as_str()
        .unwrap_or("x")
        .parse()?;
    assert!(from < to, "the gap names the skipped raw range");
    let kinds: Vec<_> = zero
        .controls
        .iter()
        .map(|record| at(record, "/params/kind"))
        .collect();
    let after_gap = kinds.iter().skip_while(|kind| **kind != "gap").nth(1);
    assert_eq!(after_gap.copied(), Some(&serde_json::json!("checkpoint")));
    assert_eq!(
        at(&zero.first("close")?, "/params/close/reason"),
        "stream_stalled"
    );
    // And the connection that stopped reading lost its interact lease, so a new
    // interact attach is ready rather than held.
    fx.journal.revoked("hung").await?;
    let next = Viewer::open(
        fx.connect("owner").await?,
        &open_params("s-4", "tui", "interact"),
        true,
    )
    .await?;
    assert_eq!(at(&next.result, "/result/outcome"), "ready");
    assert!(fx.backlog.total()? <= 64 * 1024 * 1024);
    drop(hung);
    fx.close().await
}
