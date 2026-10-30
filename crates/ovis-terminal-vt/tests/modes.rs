//! Structured, revisioned terminal modes (IDEAL-046, GAP-066).

use ovis_terminal_vt::{
    Geometry, HistoryLimit, KeyEncoding, ModifyOtherKeys, MouseEncoding, MouseTracking, PasteMode,
    ScreenBuffer, Vt, VtConfig,
};
use serde::Deserialize;

#[cfg(test)]
fn engine() -> Vt {
    let geometry = Geometry::new(100, 10).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(50)))
}

#[derive(Deserialize)]
struct BaselinePaint {
    bytes: String,
    logged_bytes: usize,
}

#[test]
fn modes_follow_the_program_and_bump_their_revision_once_per_change() {
    let mut vt = engine();
    let start = vt.mode_revision();
    assert_eq!(
        vt.modes().paste,
        PasteMode::Plain,
        "bracketed paste is never assumed"
    );

    vt.feed(
        b"\x1b[?1049h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[>3u\x1b[>4;2m\x1b[?9001h\x1b[?1h\x1b=",
    );
    let modes = vt.modes();
    assert_eq!(modes.paste, PasteMode::Bracketed);
    assert_eq!(modes.mouse.tracking, MouseTracking::Drag);
    assert_eq!(modes.mouse.encoding, MouseEncoding::Sgr);
    assert_eq!(modes.keyboard.kitty_flags, 3);
    assert_eq!(modes.keyboard.modify_other_keys, ModifyOtherKeys::All);
    assert!(modes.keyboard.win32_input);
    assert_eq!(modes.keyboard.cursor_keys, KeyEncoding::Application);
    assert_eq!(modes.keyboard.keypad, KeyEncoding::Application);
    assert_eq!(modes.screen.buffer, ScreenBuffer::Alternate);
    assert_eq!(
        vt.mode_revision().get(),
        start.get() + 9,
        "nine sequences, nine changes"
    );

    vt.feed(b"plain text does not touch modes\r\n");
    assert_eq!(vt.mode_revision().get(), start.get() + 9);
    vt.feed(b"\x1b[?2004l");
    assert_eq!(vt.modes().paste, PasteMode::Plain);
    assert_eq!(vt.mode_revision().get(), start.get() + 10);
}

#[test]
fn kitty_stack_is_per_screen_and_survives_a_round_trip() {
    let mut vt = engine();
    vt.feed(b"\x1b[>1u\x1b[?1049h\x1b[>8u");
    assert_eq!(vt.modes().keyboard.kitty_flags, 8);
    vt.feed(b"\x1b[?1049l");
    assert_eq!(vt.modes().keyboard.kitty_flags, 1);
    vt.feed(b"\x1b[<u");
    assert_eq!(vt.modes().keyboard.kitty_flags, 0);
}

#[test]
fn herdr_attach_preamble_shows_the_bracketed_paste_it_forces() {
    let fixture: BaselinePaint = serde_json::from_str(include_str!(
        "../../../probes/vt/hc-s09-first-paint-prefix.json"
    ))
    .expect("fixture parses");
    assert_eq!(fixture.bytes.len(), fixture.logged_bytes);
    let mut vt = engine();
    vt.feed(fixture.bytes.as_bytes());
    assert_eq!(
        vt.modes().paste,
        PasteMode::Bracketed,
        "hc forces 2004h on every attach"
    );
    let screen = vt.screen();
    let texts: Vec<&str> = screen.lines.iter().map(|line| line.text.as_str()).collect();
    assert!(
        texts.starts_with(&["bash-5.3$ echo HELLO_SMOKE_42", "HELLO_SMOKE"]),
        "{texts:?}"
    );

    let mut fresh = engine();
    fresh.feed(b"bash-5.3$ echo HELLO_SMOKE_42\r\nHELLO_SMOKE");
    let paint = fresh.first_paint();
    let mut attached = engine();
    attached.feed(&paint.bytes);
    assert_eq!(
        attached.modes().paste,
        PasteMode::Plain,
        "our first paint keeps the program's mode"
    );
    assert_eq!(paint.modes.paste, PasteMode::Plain);
    assert_eq!(attached.screen().lines, fresh.screen().lines);
}
