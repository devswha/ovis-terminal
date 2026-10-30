//! Reads never mutate the engine and always state their range, revision and
//! completeness (IDEAL-035, IDEAL-036, IDEAL-047).

use ovis_terminal_vt::{
    ContentRevision, Geometry, HistoryCompleteness, HistoryLimit, HistoryRead,
    HistoryUnavailableReason, LineSpan, ReadSource, SearchMatch, Vt, VtConfig,
};

#[cfg(test)]
fn engine(history: u32) -> Vt {
    let geometry = Geometry::new(20, 3).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(history)))
}

#[cfg(test)]
fn fed(history: u32) -> Vt {
    let mut vt = engine(history);
    for line in 0..8 {
        vt.feed(format!("line {line}\r\n").as_bytes());
    }
    vt
}

#[test]
fn reads_leave_state_untouched() {
    let vt = fed(50);
    let before = vt.export_checkpoint();
    let _screen = vt.screen();
    let _history = vt.history(LineSpan::new(0, 100));
    let _paint = vt.first_paint();
    let _found = vt.search("line", vt.revision()).expect("current revision");
    assert_eq!(vt.export_checkpoint(), before);
    assert_eq!(vt.screen().source, ReadSource::Emulator);
}

#[test]
fn history_reports_range_truncation_and_completeness() {
    let vt = fed(50);
    let HistoryRead::Lines {
        requested,
        returned,
        total_lines,
        truncated,
        completeness,
        lines,
        ..
    } = vt.history(LineSpan::new(4, 10))
    else {
        panic!("primary screen has history");
    };
    assert_eq!(
        (requested, returned, total_lines, truncated),
        (LineSpan::new(4, 10), LineSpan::new(4, 2), 6, true)
    );
    assert_eq!(completeness, HistoryCompleteness::Complete);
    let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(texts, vec!["line 4", "line 5"]);

    let small = fed(4);
    let HistoryRead::Lines {
        completeness,
        total_lines,
        ..
    } = small.history(LineSpan::new(0, 10))
    else {
        panic!("primary screen has history");
    };
    assert_eq!(
        (total_lines, completeness),
        (4, HistoryCompleteness::LimitReached { limit: 4 })
    );
}

#[test]
fn alternate_screen_history_is_unavailable_with_a_reason() {
    let mut vt = fed(50);
    vt.feed(b"\x1b[?1049h");
    assert!(matches!(
        vt.history(LineSpan::new(0, 5)),
        HistoryRead::Unavailable {
            reason: HistoryUnavailableReason::AlternateScreen,
            ..
        }
    ));
}

#[test]
fn search_and_selection_are_revision_guarded() {
    let mut vt = fed(50);
    let revision = vt.revision();
    let found: Vec<(u32, u32)> = vt
        .search("line 7", revision)
        .expect("current revision")
        .iter()
        .map(|hit: &SearchMatch| (hit.line, hit.char_index))
        .collect();
    assert_eq!(found, vec![(7, 0)]);
    assert_eq!(
        vt.selection_text(6..8, revision).expect("current revision"),
        "line 6\nline 7\n"
    );
    vt.feed(b"more");
    let stale = vt
        .search("line", revision)
        .expect_err("stale revision rejected");
    assert_eq!(stale.kind(), "revision_mismatch");
    let stale_selection = vt
        .selection_text(0..1, ContentRevision::default())
        .expect_err("stale");
    assert_eq!(stale_selection.kind(), "revision_mismatch");
}
