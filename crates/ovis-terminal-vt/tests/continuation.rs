//! Bytes after an advertised offset continue the live stream: a first paint or
//! a checkpoint taken mid-sequence must reproduce the live terminal once those
//! bytes arrive, or report why it cannot. Every import route enforces the
//! checkpoint format identity.

use ovis_terminal_vt::{Checkpoint, Fidelity, Geometry, HistoryLimit, InexactReason, Vt, VtConfig};
use proptest::prelude::*;

#[cfg(test)]
fn config() -> VtConfig {
    VtConfig::new(
        Geometry::new(20, 5).expect("geometry"),
        HistoryLimit::new(20),
    )
}

/// Paints `before` into a fresh engine, then feeds `after` to both.
#[cfg(test)]
fn paint_then_continue(before: &[u8], after: &[u8]) -> (Vt, Vt, Fidelity) {
    let mut live = Vt::new(config());
    live.feed(before);
    let paint = live.first_paint();
    let mut attached = Vt::new(config());
    attached.feed(&paint.bytes);
    live.feed(after);
    attached.feed(after);
    (live, attached, paint.fidelity)
}

#[test]
fn gate_first_paint_preserves_pending_control_sequence() {
    let (live, attached, fidelity) = paint_then_continue(b"A\x1b[3", b"1mB");
    assert_eq!(fidelity, Fidelity::Exact);
    assert_eq!(
        live.screen().lines.first().map(|line| line.text.as_str()),
        Some("AB")
    );
    assert_eq!(attached.screen().lines, live.screen().lines);
    assert_eq!(attached.screen().cursor, live.screen().cursor);
}

#[test]
fn first_paint_preserves_a_split_utf8_character() {
    let (live, attached, fidelity) = paint_then_continue(b"x\xe4\xb8", b"\xad!");
    assert_eq!(fidelity, Fidelity::Exact);
    assert_eq!(
        live.screen().lines.first().map(|line| line.text.as_str()),
        Some("x\u{4e2d}!")
    );
    assert_eq!(attached.screen().lines, live.screen().lines);
}

#[test]
fn first_paint_preserves_an_open_title_osc() {
    let (live, attached, fidelity) = paint_then_continue(b"t\x1b]2;half", b" done\x07");
    assert_eq!(fidelity, Fidelity::Exact);
    assert_eq!(live.screen().title.as_deref(), Some("half done"));
    assert_eq!(attached.screen().title, live.screen().title);
}

#[test]
fn first_paint_reports_a_control_inside_an_open_sequence() {
    let mut live = Vt::new(config());
    live.feed(b"top\x1b[1;\n");
    let paint = live.first_paint();
    assert_eq!(
        paint.fidelity,
        Fidelity::Inexact {
            reasons: vec![InexactReason::ControlInsideOpenSequence]
        }
    );
}

#[test]
fn gate_serde_checkpoint_cannot_bypass_version_check() {
    let geometry = Geometry::new(20, 5).expect("geometry");
    let vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(20)));
    let mut value = serde_json::to_value(vt.export_checkpoint()).expect("json");
    *value.pointer_mut("/version").expect("version field") = serde_json::json!(0);
    let checkpoint: Checkpoint = serde_json::from_value(value).expect("public serde decoder");
    let result = Vt::from_checkpoint(&checkpoint);
    assert_eq!(
        result.expect_err("stale checkpoint must not import").kind(),
        "checkpoint_incompatible"
    );
}

#[test]
fn serde_checkpoint_from_a_foreign_engine_cannot_import() {
    let vt = Vt::new(config());
    let mut value = serde_json::to_value(vt.export_checkpoint()).expect("json");
    *value.pointer_mut("/engine").expect("engine field") = serde_json::json!("another 1.0");
    let checkpoint: Checkpoint = serde_json::from_value(value).expect("public serde decoder");
    let error = Vt::from_checkpoint(&checkpoint).expect_err("foreign engine must not import");
    assert_eq!(error.kind(), "checkpoint_incompatible");
}

#[cfg(test)]
fn token() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        "[a-z ]{1,8}".prop_map(String::into_bytes),
        Just("\u{4e2d}\u{1f600}e\u{301}".as_bytes().to_vec()),
        Just(b"\r\n".to_vec()),
        (1_u8..6, 1_u8..21).prop_map(|(row, column)| format!("\x1b[{row};{column}H").into_bytes()),
        (0_u8..108).prop_map(|code| format!("\x1b[{code}m").into_bytes()),
        Just(b"\x1b[38;2;10;20;30m".to_vec()),
        Just(b"\x1b[K\x1b[2P\x1b[3@".to_vec()),
        Just(b"\x1b]7501;state=idle\x07".to_vec()),
        Just(b"\x1b]2;title\x1b\\".to_vec()),
        Just(b"\x1b[?2004h\x1b[?1002h".to_vec()),
    ]
}

proptest! {
    #[test]
    fn first_paint_at_any_byte_continues_like_the_live_stream(
        tokens in prop::collection::vec(token(), 0..30),
        cut in any::<prop::sample::Index>(),
    ) {
        let bytes = tokens.concat();
        let (before, after) = bytes.split_at(cut.index(bytes.len().saturating_add(1)));
        let (live, attached, fidelity) = paint_then_continue(before, after);
        prop_assert_eq!(fidelity, Fidelity::Exact);
        prop_assert_eq!(attached.screen().lines, live.screen().lines);
        prop_assert_eq!(attached.screen().cursor, live.screen().cursor);
        prop_assert_eq!(attached.modes(), live.modes());
        prop_assert_eq!(attached.screen().title, live.screen().title);
    }
}
