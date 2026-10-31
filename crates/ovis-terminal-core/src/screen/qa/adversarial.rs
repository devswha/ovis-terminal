//! Adversarial classes: reflow and eviction staleness, hostile screen text,
//! reads independent of a busy producer, terminal-query replies and ending.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use ovis_terminal_core::screen::{
    HistoryRequest, Recovery, ScreenBounds, ScreenError, ScreenTransition, SearchRequest,
    UnavailableReason,
};
use ovis_terminal_holder::protocol::Exit;
use ovis_terminal_vt::Geometry;

use crate::fixture::{TestResult, feed, limit, needle, nonzero, numbered_lines, output, screen};

#[test]
fn reflow_keeps_pinned_pages_and_rejects_unpinned_revisions() -> TestResult {
    // Given: a paged history read pinned at revision R before a resize.
    let screen = screen("terminal:reflow", ScreenBounds::DEFAULT)?;
    feed(&screen, &numbered_lines(30))?;
    let first = screen
        .history(&HistoryRequest::new(limit(10)))
        .expect("history");
    let cursor = first.next.expect("more pages");

    // When: the grid reflows to a narrower width.
    let after = screen
        .resize(Geometry::new(8, 5).expect("geometry"))
        .expect("resize");

    // Then: the pinned continuation is served at R; the live revision moved.
    assert!(after > first.meta.revision);
    let second = screen
        .history(&HistoryRequest::new(limit(10)).at(cursor))
        .expect("pinned page survives reflow");
    assert_eq!(second.meta.revision, first.meta.revision);
    assert_eq!(second.meta.live.revision, after);
    assert_eq!(
        second.lines.first().map(|line| line.text.as_str()),
        Some("line 10")
    );
    Ok(())
}

#[test]
fn screen_text_and_osc_bytes_never_drive_transitions_or_status() -> TestResult {
    // Given: a program that prints status-looking text and a status OSC.
    let screen = screen("terminal:injection", ScreenBounds::DEFAULT)?;

    // When: the bytes are ingested.
    let ingested = feed(
        &screen,
        b"history_available status=idle\r\n\x1b]7501;status=idle\x07$ ",
    )?;

    // Then: no screen transition fires and the OSC record is only forwarded.
    assert_eq!(ingested.transitions.len(), 0);
    assert_eq!(ingested.osc.len(), 1);
    let view = screen.screen().expect("screen");
    assert_eq!(
        view.lines.first().map(|line| line.text.as_str()),
        Some("history_available status=idle")
    );
    Ok(())
}

#[test]
fn reads_complete_while_a_producer_keeps_feeding() -> TestResult {
    // Given: a producer thread ingesting continuously.
    let screen = screen(
        "terminal:busy",
        ScreenBounds::DEFAULT.with_pinned_snapshots(nonzero(64)),
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = mpsc::channel();
    let producer = thread::spawn({
        let screen = screen.clone();
        let stop = Arc::clone(&stop);
        move || {
            let mut sent = false;
            while !stop.load(Ordering::Acquire) {
                feed(&screen, &numbered_lines(200)).expect("contiguous producer output");
                if !sent {
                    sent = started_tx.send(()).is_ok();
                }
            }
        }
    });
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("producer started");

    // When: pages are read at a pinned revision while output keeps arriving.
    let mut revisions = Vec::new();
    for _ in 0..20 {
        let first = screen
            .history(&HistoryRequest::new(limit(7)))
            .expect("live page");
        let next = first.next.expect("continuation");
        let second = screen
            .history(&HistoryRequest::new(limit(7)).at(next))
            .expect("pinned continuation never reports a transient conflict");
        assert_eq!(second.meta.revision, first.meta.revision);
        revisions.push(first.meta.revision);
    }
    stop.store(true, Ordering::Release);
    producer.join().expect("producer");

    // Then: every read succeeded and revisions only move forward.
    assert!(revisions.is_sorted());
    Ok(())
}

#[test]
fn alternate_screen_history_is_unavailable_until_the_named_transition() -> TestResult {
    // Given: a full-screen program.
    let screen = screen("terminal:alt", ScreenBounds::DEFAULT)?;
    let entered = feed(&screen, b"before\r\n\x1b[?1049hfull screen")?;

    // When: history is read while the alternate screen is active.
    let refused = screen
        .history(&HistoryRequest::new(limit(5)))
        .expect_err("alternate screen has no history");

    // Then: the rejection names the transition that ends it, and leaving fires it.
    assert_eq!(refused.kind(), "history_unavailable");
    assert!(matches!(
        refused,
        ScreenError::HistoryUnavailable { ref unavailability, .. }
            if unavailability.reason == UnavailableReason::AlternateScreen
            && unavailability.recovery == Recovery::HistoryAvailableTransition
    ));
    assert!(matches!(
        entered.transitions.as_slice(),
        [ScreenTransition::HistoryUnavailable { .. }]
    ));
    let left = feed(&screen, b"\x1b[?1049l")?;
    assert!(matches!(
        left.transitions.as_slice(),
        [ScreenTransition::HistoryAvailable { .. }]
    ));
    assert!(screen.history(&HistoryRequest::new(limit(5))).is_ok());
    Ok(())
}

#[test]
fn ending_in_the_alternate_screen_is_a_permanent_reason() -> TestResult {
    // Given: a program that exits while full-screen.
    let screen = screen("terminal:alt-exit", ScreenBounds::DEFAULT)?;
    feed(&screen, b"\x1b[?1049hbye")?;
    let final_offset = screen.live().expect("live").offset.get();

    // When: the exit is observed.
    let transitions = screen
        .finish(&Exit {
            target: screen.target().clone(),
            code: Some(0),
            signal: None,
            exit_unknown: None,
            io_failure: None,
            final_offset,
        })
        .expect("exit at the final offset");

    // Then: history is permanently unavailable and more output is refused.
    assert!(matches!(
        transitions.as_slice(),
        [ScreenTransition::HistoryUnavailable { unavailability, .. }]
            if unavailability.recovery == Recovery::Permanent
    ));
    let refused = screen
        .history(&HistoryRequest::new(limit(5)))
        .expect_err("permanent");
    assert!(matches!(
        refused,
        ScreenError::HistoryUnavailable { ref unavailability, .. }
            if unavailability.reason == UnavailableReason::EndedInAlternateScreen
    ));
    let late = screen
        .ingest(&output(screen.target(), final_offset, b"late"))
        .expect_err("ended");
    assert_eq!(late.kind(), "terminal_ended");
    Ok(())
}

#[test]
fn output_from_another_incarnation_is_refused() -> TestResult {
    // Given: a screen bound to one terminal incarnation.
    let screen = screen("terminal:owner", ScreenBounds::DEFAULT)?;

    // When: a chunk for another terminal arrives.
    let foreign = crate::fixture::target("terminal:other");
    let error = screen
        .ingest(&output(&foreign, 0, b"x"))
        .expect_err("foreign");

    // Then: it is a typed target mismatch and nothing was read.
    assert_eq!(error.kind(), "target_mismatch");
    let found = screen
        .search(&SearchRequest::new(needle("x")?, limit(1)))
        .expect("search");
    assert_eq!(found.matches.len(), 0);
    Ok(())
}

#[test]
fn query_replies_overflow_visibly_and_are_taken_once() -> TestResult {
    // Given: a reply lane with room for two replies.
    let screen = screen(
        "terminal:queries",
        ScreenBounds::DEFAULT.with_reply_capacity(nonzero(2)),
    )?;

    // When: the program asks three cursor-position queries.
    let ingested = feed(&screen, b"\x1b[6n\x1b[6n\x1b[6n")?;

    // Then: two replies are queued once and the third is counted as dropped.
    assert_eq!(ingested.queued_replies, 2);
    let batch = screen.drain_replies().expect("drain");
    assert_eq!(batch.replies.len(), 2);
    assert_eq!(batch.dropped, 1);
    let again = screen.drain_replies().expect("drain");
    assert_eq!(again.replies.len(), 0);
    assert_eq!(again.dropped, 0);
    Ok(())
}
