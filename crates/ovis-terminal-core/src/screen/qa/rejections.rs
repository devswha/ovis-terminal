//! Failure path: evicted cursors and stale revisions are typed rejections,
//! never empty complete history.

use std::io::{self, Write};

use ovis_terminal_core::screen::{
    HistoryRequest, OutputRequest, PageCursor, ScreenBounds, ScreenError, SearchRequest,
    SelectionRequest,
};
use ovis_terminal_vt::LineSpan;

use crate::fixture::{
    TestResult, feed, limit, needle, nonzero, numbered_lines, offset, output, screen,
};

const fn small_bounds() -> ScreenBounds {
    ScreenBounds::DEFAULT
        .with_retained_output(nonzero(64))
        .with_checkpoint_interval(nonzero(32))
        .with_work_budget(nonzero(16))
        .with_pinned_snapshots(nonzero(1))
}

/// Evicted cursors, stale revisions and output gaps are typed rejections.
///
/// # Errors
///
/// Returns holder, screen or encoding failures.
///
/// # Panics
///
/// Panics when an observed read violates the acceptance contract.
pub fn failure() -> TestResult {
    // Given: a screen whose raw retention covers only the newest 64 bytes.
    let screen = screen("terminal:evicted", small_bounds())?;
    let bytes = numbered_lines(60);
    feed(&screen, &bytes)?;
    let head = u64::try_from(bytes.len())?;

    // When: a viewer asks for an evicted output cursor.
    let Err(evicted) = screen.output(&OutputRequest::new(offset(0), limit(16))) else {
        return Err("offset zero was evicted, yet the read succeeded".into());
    };

    // Then: history_gap names the retained range and a checkpoint inside it.
    let ScreenError::HistoryGap {
        requested,
        retained,
        checkpoint,
    } = &evicted
    else {
        return Err(format!("expected history_gap, got {evicted:?}").into());
    };
    assert_eq!(evicted.kind(), "history_gap");
    assert_eq!(*requested, offset(0));
    assert_eq!(retained.head, offset(head));
    assert_eq!(retained.floor, offset(head.saturating_sub(64)));
    let mark = checkpoint
        .as_ref()
        .ok_or("a checkpoint covers the retained range")?;
    assert!(mark.offset >= retained.floor && mark.offset <= retained.head);

    // Given: a read pinned revision R, then output and a second read evicted that pin.
    let stale = screen.screen()?.meta.revision;
    feed(&screen, b"more output\r\n")?;
    let current = screen.screen()?.meta.revision;

    // When: history, search and selection name the evicted revision.
    let page = screen.history(&HistoryRequest::new(limit(5)).at(PageCursor::new(stale, 0)));
    let search = screen.search(&SearchRequest::new(needle("line")?, limit(5)).at(stale));
    let selection = screen.selection(&SelectionRequest::new(stale, LineSpan::new(0, 1)));

    // Then: each is stale_revision with the current revision, not an empty success.
    for rejected in [page.err(), search.err(), selection.err()] {
        let error = rejected.ok_or("stale revision must be rejected")?;
        assert_eq!(error.kind(), "stale_revision");
        assert!(matches!(
            error,
            ScreenError::StaleRevision { requested, current: now }
                if requested == stale && now == current
        ));
    }

    // Then: a non-contiguous holder chunk is refused without moving the screen.
    let before = screen.live()?.offset;
    let Err(gap) = screen.ingest(&output(
        screen.target(),
        before.get().saturating_add(1),
        b"x",
    )) else {
        return Err("a gap must be refused".into());
    };
    assert_eq!(gap.kind(), "output_gap");
    assert_eq!(screen.live()?.offset, before);
    writeln!(
        io::stdout(),
        "{}",
        serde_json::json!({
            "history_gap": evicted.kind(), "retained": retained, "checkpoint": checkpoint,
            "stale": {"kind": "stale_revision", "requested": stale, "current": current},
            "output_gap": gap.kind(),
        })
    )?;
    Ok(())
}
