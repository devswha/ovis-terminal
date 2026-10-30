//! First paint, fed to a fresh engine, must reproduce the live engine's
//! visible cells, cursors, modes and terminal-wide state.

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::Line;
use alacritty_terminal::term::cell::Cell;
use proptest::prelude::*;

use crate::engine::Vt;
use crate::engine::fidelity::{Fidelity, InexactReason};
use crate::engine::modes::ScreenBuffer;
use crate::engine::units::{Geometry, HistoryLimit, VtConfig};

fn engine(columns: u16, rows: u16) -> Vt {
    let geometry = Geometry::new(columns, rows).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(100)))
}

/// Feeds the live engine's paint to a fresh engine and lists every state that
/// differs, except a saved cursor the paint declared it had to spend.
fn paint_differences(live: &Vt) -> Vec<String> {
    let paint = live.first_paint();
    let mut fresh = Vt::new(live.config);
    fresh.feed(&paint.bytes);
    let spent = match &paint.fidelity {
        Fidelity::Exact => false,
        Fidelity::Inexact { reasons } => reasons == &[InexactReason::SavedCursorNotPaintable],
    };
    same_view(live, &fresh)
        .into_iter()
        .filter(|name| !(spent && name.starts_with("saved cursor")))
        .collect()
}

fn visible_rows_match(left: &Grid<Cell>, right: &Grid<Cell>) -> Option<String> {
    (0..=left.bottommost_line().0)
        .map(Line)
        .find(|&line| left[line] != right[line])
        .map(|line| {
            format!(
                "row {} differs:\n live {:?}\n paint {:?}",
                line.0, left[line], right[line]
            )
        })
}

fn same_view(live: &Vt, paint: &Vt) -> Vec<String> {
    let mut differences: Vec<String> = visible_rows_match(live.term.grid(), paint.term.grid())
        .into_iter()
        .collect();
    if live.modes.screen.buffer == ScreenBuffer::Alternate {
        differences.extend(visible_rows_match(&live.inactive, &paint.inactive));
    }
    let (live_grid, paint_grid) = (live.term.grid(), paint.term.grid());
    let checks = [
        ("cursor", live_grid.cursor == paint_grid.cursor),
        (
            "saved cursor point",
            live_grid.saved_cursor.point == paint_grid.saved_cursor.point,
        ),
        (
            "saved cursor pen",
            live_grid.saved_cursor.template == paint_grid.saved_cursor.template,
        ),
        ("mode bits", live.term.mode() == paint.term.mode()),
        ("structured modes", live.modes == paint.modes),
        ("tab stops", live.shadow.tab_stops == paint.shadow.tab_stops),
        (
            "scroll region",
            live.shadow.scroll_region == paint.shadow.scroll_region,
        ),
        ("title", live.shadow.title == paint.shadow.title),
        (
            "keyboard stack",
            live.shadow.keyboard_stack == paint.shadow.keyboard_stack,
        ),
        (
            "active charset",
            live.shadow.active_charset == paint.shadow.active_charset,
        ),
        (
            "cursor style",
            live.term.cursor_style() == paint.term.cursor_style(),
        ),
        (
            "palette",
            (0..alacritty_terminal::term::color::COUNT)
                .all(|index| live.term.colors()[index] == paint.term.colors()[index]),
        ),
    ];
    differences.extend(
        checks
            .iter()
            .filter(|(_, holds)| !holds)
            .map(|(name, _)| (*name).to_owned()),
    );
    differences
}

#[test]
fn primary_screen_paint_matches_live_state() {
    let mut live = engine(20, 6);
    live.feed(b"\x1b]4;3;rgb:12/34/56\x1b\\\x1b]2;editor\x07\x1b[3g\x1b[3G\x1bH\x1b[11G\x1bH\r");
    live.feed(
        "\x1b[1;4:3;38;2;1;2;3;48;5;17;58;5;9mwide \u{4e2d}\u{6587}\x1b[0m e\u{301}\r\n".as_bytes(),
    );
    live.feed(b"\x1b]8;id=doc;https://example.test\x1b\\link\x1b]8;;\x1b\\\ta\tb\r\n");
    live.feed(b"\x1b[7mwrapping text that is long\x1b[0m\r\n\x1b(0lqk\x1b(B\x0e\x1b[>5u\x1b[>4;1m");
    live.feed(b"\x1b[?2004h\x1b[?1003h\x1b[?1006h\x1b[?25l\x1b[5 q\x1b[2;5r\x1b[?6h\x1b[3;2H\x1b[33mpen\x1b7");
    live.feed(b"\x1b[1;1H\x1b[6;1Habcdefghijklmnopqrst");
    assert!(
        live.term.grid().cursor.input_needs_wrap,
        "fixture ends with a pending wrap"
    );
    assert_eq!(paint_differences(&live), Vec::<String>::new());
}

#[test]
fn alternate_screen_paint_restores_both_screens() {
    let mut live = engine(16, 5);
    live.feed(
        "shell $ ls\r\n\u{4e2d}\u{6587}.txt\r\n\x1b[?1049h\x1b[Hvim\x1b[2;3H~ e\u{301}\x1b[?1h"
            .as_bytes(),
    );
    assert_eq!(live.modes.screen.buffer, ScreenBuffer::Alternate);
    assert_eq!(paint_differences(&live), Vec::<String>::new());
}

#[test]
fn paint_after_resize_matches_reflowed_state() {
    let mut live = engine(12, 4);
    live.feed(b"0123456789abcdefghij\r\nshort\r\n");
    live.resize(Geometry::new(7, 5).expect("valid geometry"));
    assert_eq!(paint_differences(&live), Vec::<String>::new());
}

#[test]
fn cursor_outside_origin_region_spends_saved_cursor_and_says_so() {
    let mut live = engine(10, 6);
    live.feed(b"\x1b[1;1H\x1b7\x1b[?6h\x1b[2;4r\x1b8ab");
    let paint = live.first_paint();
    assert_eq!(
        paint.fidelity,
        Fidelity::Inexact {
            reasons: vec![InexactReason::SavedCursorNotPaintable]
        }
    );
    assert_eq!(paint_differences(&live), Vec::<String>::new());
}

fn token() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        "[a-z ]{1,12}".prop_map(String::into_bytes),
        Just("\u{4e2d}\u{6587}".as_bytes().to_vec()),
        Just("e\u{301}".as_bytes().to_vec()),
        Just(b"\r\n".to_vec()),
        Just(b"\t".to_vec()),
        (0_u8..8, 0_u8..8).prop_map(|(row, column)| format!("\x1b[{row};{column}H").into_bytes()),
        (0_u8..110).prop_map(|code| format!("\x1b[{code}m").into_bytes()),
        prop::sample::select(vec![
            "1049", "2004", "1000", "1002", "1006", "25", "1", "7", "6"
        ])
        .prop_flat_map(|mode| prop::bool::ANY.prop_map(move |on| {
            format!("\x1b[?{mode}{}", if on { 'h' } else { 'l' }).into_bytes()
        })),
        Just(b"\x1b[2;4r".to_vec()),
        Just(b"\x1b[K".to_vec()),
        Just(b"\x1b[2J".to_vec()),
        Just(b"\x1bH".to_vec()),
        Just(b"\x1b[g".to_vec()),
        Just(b"\x1b7".to_vec()),
        Just(b"\x1b8".to_vec()),
        Just(b"\x1b[>3u".to_vec()),
        Just(b"\x1b(0".to_vec()),
        Just(b"\x1bc".to_vec()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn paint_reproduces_arbitrary_streams(tokens in prop::collection::vec(token(), 0..60)) {
        let mut live = engine(10, 6);
        for bytes in &tokens {
            live.feed(bytes);
        }
        prop_assert_eq!(paint_differences(&live), Vec::<String>::new());
    }
}
