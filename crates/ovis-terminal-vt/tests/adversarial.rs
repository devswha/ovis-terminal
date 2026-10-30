//! Hostile output and hostile checkpoints are rejected with typed outcomes,
//! never a panic, a fabricated record or an unchecked import.

use ovis_terminal_vt::{
    Checkpoint, Fidelity, Geometry, HistoryLimit, InexactReason, MIN_COLUMNS, OSC_FIELD_LIMIT,
    OSC_RECORD_LIMIT, OscContent, OscFault, OscTerminator, PasteMode, Vt, VtConfig,
};

#[cfg(test)]
fn engine() -> Vt {
    let geometry = Geometry::new(30, 5).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(20)))
}

#[cfg(test)]
fn set(json: &mut serde_json::Value, pointer: &str, value: serde_json::Value) {
    *json.pointer_mut(pointer).expect("checkpoint field exists") = value;
}

#[test]
fn kitty_stack_overflow_is_refused_instead_of_crashing_the_engine() {
    let mut vt = engine();
    let pushes = b"\x1b[>1u".repeat(5000);
    vt.feed(&pushes);
    vt.feed(b"after");
    assert_eq!(vt.modes().keyboard.kitty_flags, 1);
    assert_eq!(
        vt.screen().lines.first().map(|line| line.text.as_str()),
        Some("after")
    );
    let restored = Vt::from_checkpoint(&vt.export_checkpoint()).expect("bounded stack imports");
    assert_eq!(restored.modes(), vt.modes());
}

#[test]
fn osc_payload_at_the_limit_is_kept_and_one_byte_more_is_oversize() {
    let mut vt = engine();
    let at_limit = format!("\x1b]7501;{}\x07", "a".repeat(OSC_RECORD_LIMIT - 5));
    let over = format!("\x1b]7501;{}\x07", "a".repeat(OSC_RECORD_LIMIT - 4));
    let records = vt.feed(format!("{at_limit}{over}").as_bytes()).osc;
    let [kept, oversize] = records.as_slice() else {
        panic!("two records expected, got {records:?}")
    };
    assert!(matches!(
        kept.content,
        OscContent::Fields { number: 7501, .. }
    ));
    assert!(
        matches!(oversize.content, OscContent::Oversize { number: Some(7501), bytes } if usize::try_from(bytes) == Ok(OSC_RECORD_LIMIT + 1))
    );
}

#[test]
fn emulator_owned_osc_numbers_never_leak_as_records() {
    let mut vt = engine();
    let records = vt
        .feed(b"\x1b]52;c;c2VjcmV0\x07\x1b]2;title\x07\x1b]8;;http://x\x1b\\")
        .osc;
    assert!(
        records.is_empty(),
        "clipboard, title and hyperlink stay inside the emulator"
    );
}

#[test]
fn fake_status_and_paste_text_are_only_screen_text() {
    let mut vt = engine();
    let outcome = vt.feed(b"state=working\r\n\\e[?2004h approve? [y/n]");
    assert!(outcome.osc.is_empty() && outcome.queries.is_empty());
    assert_eq!(vt.modes().paste, PasteMode::Plain);
}

#[test]
fn checkpoint_contradicting_its_geometry_is_rejected() {
    let mut vt = engine();
    vt.feed(b"hello");
    let mut json: serde_json::Value =
        serde_json::from_slice(&vt.export_checkpoint().to_bytes().expect("encodes")).expect("json");
    set(&mut json, "/config/geometry/columns", serde_json::json!(80));
    let bytes = serde_json::to_vec(&json).expect("encodes");
    let checkpoint = Checkpoint::from_bytes(&bytes).expect("still decodes");
    let error = Vt::from_checkpoint(&checkpoint).expect_err("inconsistent checkpoint rejected");
    assert_eq!(error.kind(), "checkpoint_inconsistent");

    set(&mut json, "/config/geometry/columns", serde_json::json!(1));
    let narrow = serde_json::to_vec(&json).expect("encodes");
    let error = Checkpoint::from_bytes(&narrow).expect_err("one column cannot decode");
    assert_eq!(error.kind(), "checkpoint_malformed");
}

#[test]
fn degenerate_geometry_is_a_typed_error() {
    for (columns, rows) in [(0, 24), (1, 24), (80, 0)] {
        let error = Geometry::new(columns, rows).expect_err("degenerate geometry rejected");
        assert_eq!(error.kind(), "invalid_geometry");
    }
}

#[test]
fn narrowest_grid_survives_wide_characters() {
    let geometry = Geometry::new(MIN_COLUMNS, 2).expect("valid geometry");
    let mut vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(5)));
    vt.feed("\u{4e2d}\u{6587}x\u{1f600}".as_bytes());
    let restored = Vt::from_checkpoint(&vt.export_checkpoint()).expect("imports");
    assert_eq!(restored.screen().lines, vt.screen().lines);
}

#[test]
fn gate_oversize_osc_after_sixteen_params_is_not_success() {
    let mut vt = engine();
    let bytes = format!("\x1b]7501;{}{}\x07", "a;".repeat(15), "z".repeat(5000));
    let payload = u32::try_from(bytes.len() - "\x1b]\x07".len()).expect("fits");
    let result = vt.feed(bytes.as_bytes());
    assert_eq!(
        result.osc.first().map(|record| &record.content),
        Some(&OscContent::Oversize {
            number: Some(7501),
            bytes: payload
        })
    );
}

#[test]
fn more_fields_than_the_parser_keeps_is_malformed_not_truncated_fields() {
    let mut vt = engine();
    let at_limit = format!("\x1b]7501{}\x07", ";f".repeat(OSC_FIELD_LIMIT - 1));
    let over = format!("\x1b]7501{}\x07", ";f".repeat(OSC_FIELD_LIMIT));
    let records = vt.feed(format!("{at_limit}{over}").as_bytes()).osc;
    let [kept, dropped] = records.as_slice() else {
        panic!("two records, got {records:?}")
    };
    assert!(
        matches!(&kept.content, OscContent::Fields { fields, .. } if fields.len() == OSC_FIELD_LIMIT - 1)
    );
    assert_eq!(
        dropped.content,
        OscContent::Malformed {
            fault: OscFault::TooManyFields
        }
    );
}

#[test]
fn ten_mib_unterminated_osc_is_cut_at_the_parser_bound() {
    let mut vt = engine();
    vt.feed(b"\x1b]7501;state=");
    let chunk = vec![b'z'; 64 * 1024];
    let mut records = Vec::new();
    for _ in 0..160 {
        records.extend(vt.feed(&chunk).osc);
    }
    let [record] = records.as_slice() else {
        panic!("one truncated record, got {records:?}")
    };
    assert_eq!(record.terminator, OscTerminator::Truncated);
    assert!(matches!(
        record.content,
        OscContent::Oversize {
            number: Some(7501),
            ..
        }
    ));

    let checkpoint = vt.export_checkpoint();
    assert_eq!(
        checkpoint.pending_bytes(),
        b"",
        "the parser holds nothing of the cut string"
    );
    assert_eq!(
        checkpoint.fidelity(),
        &Fidelity::Exact,
        "the discard state travels in the checkpoint"
    );
    assert_eq!(
        vt.first_paint().fidelity,
        Fidelity::Inexact {
            reasons: vec![InexactReason::OpenSequenceTooLong]
        }
    );
    let mut restored = Vt::from_checkpoint(&checkpoint).expect("imports");

    let tail = b"zzzz\x07ok";
    vt.feed(tail);
    restored.feed(tail);
    assert_eq!(
        vt.screen().lines.first().map(|line| line.text.as_str()),
        Some("ok")
    );
    assert_eq!(restored.export_checkpoint(), vt.export_checkpoint());
}

#[test]
fn oversized_dcs_string_is_discarded_until_its_terminator() {
    let mut vt = engine();
    vt.feed(b"\x1bPq");
    vt.feed(&vec![b'#'; 70 * 1024]);
    vt.feed(b"\x1b\\after");
    assert_eq!(
        vt.screen().lines.first().map(|line| line.text.as_str()),
        Some("after")
    );
    assert_eq!(vt.export_checkpoint().fidelity(), &Fidelity::Exact);
}
