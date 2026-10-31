//! The first paint is fenced ahead of live bytes even while credit holds it back,
//! and an exited terminal answers attach with a typed `ended` record.

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::StreamLimits;
use ovis_terminal_vt::Vt;

use super::TestResult;
use super::fixture::{Fixture, vt_config};
use super::socket::at;
use super::viewer::{Viewer, open_params};

/// Far smaller than the first paint, so the paint is still blocked when live bytes arrive.
const WINDOW: usize = 64;

fn screen_text() -> Vec<u8> {
    (0..8_u8)
        .flat_map(|row| {
            format!(
                "\x1b[3{}mrow {row} painted once before attach\x1b[0m\r\n",
                row % 8
            )
            .into_bytes()
        })
        .collect()
}

/// Acknowledges in window-sized steps until `total` bytes were received.
async fn drain(viewer: &mut Viewer, total: usize) -> TestResult {
    while viewer.received.len() < total {
        let target = viewer.received.len().saturating_add(WINDOW).min(total);
        viewer.acknowledge().await?;
        viewer
            .until(|viewer| viewer.received.len() >= target)
            .await?;
    }
    Ok(())
}

pub(crate) async fn paint_is_fenced_ahead_of_live_bytes_under_credit_pressure() -> TestResult {
    let limits = StreamLimits::advertised(&Limits::v1()).with_credit_window(64);
    let mut fx = Fixture::new(limits, 64 * 1024 * 1024).await?;
    // Given a screen painted once, and the paint a reference terminal would send for it.
    fx.publish(&screen_text())?;
    let mut before = Vt::new(vt_config()?);
    drop(before.feed(&fx.raw));
    let paint = before.first_paint().bytes;
    assert!(
        paint.len() > WINDOW.saturating_mul(4),
        "the paint must exceed the window"
    );
    // When a viewer attaches without acknowledging, so its paint is blocked on credit.
    let client = fx.connect("owner").await?;
    let mut viewer = Viewer::open(client, &open_params("fence-1", "tui", "observe"), false).await?;
    viewer
        .until(|viewer| viewer.received.len() >= WINDOW)
        .await?;
    assert_eq!(viewer.received.len(), WINDOW, "only the window was sent");
    // And live output arrives while the paint is still undelivered.
    let live = b"\x1b[2;5HLIVE-AFTER-FENCE\r\nnext line\r\n".repeat(20);
    fx.publish(&live)?;
    // Then acknowledging step by step yields the whole paint first, then the live bytes.
    drain(&mut viewer, paint.len().saturating_add(live.len())).await?;
    assert_eq!(
        viewer.received.get(..paint.len()),
        Some(paint.as_slice()),
        "paint first"
    );
    assert_eq!(
        viewer.received.get(paint.len()..),
        Some(live.as_slice()),
        "live bytes after the fence"
    );
    let first = viewer.controls.first().ok_or("no control record")?;
    assert_eq!(at(first, "/params/kind"), "checkpoint");
    assert_eq!(at(first, "/params/stream_offset"), "0");
    // And the client's screen equals a terminal that saw every raw byte.
    let mut continuous = Vt::new(vt_config()?);
    drop(continuous.feed(&fx.raw));
    let screen = viewer.screen()?;
    assert_eq!(screen.screen().lines, continuous.screen().lines);
    assert_eq!(screen.screen().cursor, continuous.screen().cursor);
    assert_eq!(screen.modes(), continuous.modes());
    fx.close().await
}

pub(crate) async fn an_exited_terminal_answers_ended() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), 64 * 1024 * 1024).await?;
    fx.publish(b"build finished\r\n")?;
    fx.exit(3)?;
    let raw_end = fx.raw.len().to_string();
    // When anyone attaches after the exit, even asking for interact.
    let client = fx.connect("owner").await?;
    let mut viewer = Viewer::open(client, &open_params("ended-1", "tui", "interact"), true).await?;
    // Then the outcome is typed `ended` with the final offset and an observe-only paint.
    let result = &viewer.result;
    assert_eq!(at(result, "/result/outcome"), "ended");
    assert_eq!(at(result, "/result/reason"), "exited");
    assert_eq!(at(result, "/result/final_output_offset"), raw_end.as_str());
    assert!(
        at(result, "/result/lease").is_null(),
        "an ended terminal grants no lease"
    );
    assert_eq!(
        at(result, "/result/first_paint/output_offset"),
        raw_end.as_str()
    );
    viewer
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let close = viewer.first("close")?;
    assert_eq!(at(&close, "/params/close/reason"), "exited");
    assert_eq!(at(&close, "/params/close/exit_code"), 3);
    assert_eq!(
        viewer.kinds("input_ready").len(),
        0,
        "an ended stream is never input-ready"
    );
    let mut continuous = Vt::new(vt_config()?);
    drop(continuous.feed(&fx.raw));
    assert_eq!(viewer.screen()?.screen().lines, continuous.screen().lines);
    fx.close().await
}
