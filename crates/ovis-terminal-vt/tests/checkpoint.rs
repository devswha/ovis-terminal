//! Checkpoint exactness: a checkpoint plus the bytes retained after it must
//! reproduce an always-live engine, and stale or hostile input must be rejected
//! with a typed reason instead of becoming state.

use ovis_terminal_vt::{
    Checkpoint, Fidelity, Geometry, HistoryLimit, OscContent, OscFault, OutputOffset, QueryOutcome,
    Vt, VtConfig,
};

#[cfg(test)]
fn engine() -> Vt {
    let geometry = Geometry::new(40, 10).expect("40x10 is a valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(200)))
}

/// Output a shell, a full-screen editor and their mode changes would produce,
/// ending inside an open CSI sequence (`ESC [ 3`).
#[cfg(test)]
fn before_checkpoint() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b]2;build log\x07\x1b[?2004h\x1b[>1u\x1b[>4;2m\x1b[?9001h");
    bytes.extend_from_slice(b"\x1b[?1006h\x1b[?1002h\x1b[3g\x1b[5G\x1bH\r");
    for line in 1..=24 {
        bytes.extend_from_slice(
            format!("\x1b[1;31mline {line}\x1b[0m \u{4e2d}\u{6587} e\u{301}\r\n").as_bytes(),
        );
    }
    bytes.extend_from_slice(b"\x1b[2;8r\x1b7\x1b[?1049h\x1b[2J\x1b[Hvim buffer\r\n~\r\n~");
    bytes.extend_from_slice(b"\x1b[38;5;208m\x1b[3");
    bytes
}

/// Output after the checkpoint: it completes the open CSI, leaves the
/// alternate screen, asks for the cursor position and reports an OSC record.
const AFTER_CHECKPOINT: &[u8] =
    b"8;2;1;2;3mX\x1b[?1049l\x1b8after\x1b[6n\t|tab\r\n\x1b]7501;state=idle\x07done \xe4\xb8\xad";

#[cfg(test)]
fn replies(queries: &[ovis_terminal_vt::TerminalQuery]) -> Vec<(OutputOffset, Vec<u8>)> {
    queries
        .iter()
        .filter_map(|query| match &query.outcome {
            QueryOutcome::Reply(bytes) => Some((query.offset, bytes.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn happy() {
    let before = before_checkpoint();
    let mut live = engine();
    for chunk in before.chunks(3) {
        live.feed(chunk);
    }

    let checkpoint = live.export_checkpoint();
    assert_eq!(checkpoint.fidelity(), &Fidelity::Exact);
    assert_eq!(checkpoint.pending_bytes(), b"\x1b[3");
    let encoded = checkpoint.to_bytes().expect("checkpoint encodes");
    let decoded = Checkpoint::from_bytes(&encoded).expect("checkpoint decodes");
    assert_eq!(decoded, checkpoint);
    let mut restored = Vt::from_checkpoint(&decoded).expect("checkpoint imports");

    let mut live_replies = Vec::new();
    for chunk in AFTER_CHECKPOINT.chunks(2) {
        live_replies.extend(replies(&live.feed(chunk).queries));
    }
    let restored_replies = replies(&restored.feed(AFTER_CHECKPOINT).queries);

    assert_eq!(restored.export_checkpoint(), live.export_checkpoint());
    assert_eq!(restored.screen().cursor, live.screen().cursor);
    assert_eq!(restored.modes(), live.modes());
    assert_eq!(restored.revision(), live.revision());
    assert_eq!(restored.mode_revision(), live.mode_revision());
    assert_eq!(restored.offset(), live.offset());
    assert_eq!(restored_replies, live_replies);
    assert_eq!(live_replies.len(), 1, "exactly one cursor-position reply");

    let screen = live.screen();
    let texts: Vec<&str> = screen.lines.iter().map(|line| line.text.as_str()).collect();
    let [home, _, done, ..] = texts.as_slice() else {
        panic!("screen has rows: {texts:?}")
    };
    assert!(
        home.starts_with("after16"),
        "DECRC returned home on the primary screen"
    );
    assert!(
        done.starts_with("done \u{4e2d}"),
        "split UTF-8 completed after the checkpoint"
    );
    let summary = serde_json::json!({
        "checkpoint_bytes": encoded.len(),
        "checkpoint_offset": checkpoint.offset(),
        "pending_bytes": String::from_utf8_lossy(checkpoint.pending_bytes()),
        "offset": live.offset(),
        "revision": live.revision(),
        "mode_revision": live.mode_revision(),
        "cursor": screen.cursor,
        "modes": live.modes(),
        "replies": live_replies.iter().map(|(offset, bytes)| (offset, String::from_utf8_lossy(bytes))).collect::<Vec<_>>(),
        "restored_equals_live": restored.export_checkpoint() == live.export_checkpoint(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).expect("summary serializes")
    );
}

#[test]
fn failure() {
    let mut vt = engine();
    let mut hostile = b"Status: working\r\nApprove? [y/n] \x1b]75x1;state=working\x07".to_vec();
    hostile.extend_from_slice(b"\x1b]7501;state=blocked;msg=");
    hostile.extend(std::iter::repeat_n(b'A', 5000));
    hostile.push(0x07);
    let outcome = vt.feed(&hostile);

    let contents: Vec<&OscContent> = outcome.osc.iter().map(|record| &record.content).collect();
    let [malformed, oversize] = contents.as_slice() else {
        panic!("one malformed and one oversize record, got {contents:?}")
    };
    assert!(matches!(
        malformed,
        OscContent::Malformed {
            fault: OscFault::InvalidNumber
        }
    ));
    assert!(matches!(
        oversize,
        OscContent::Oversize {
            number: Some(7501),
            ..
        }
    ));
    let text: Vec<String> = vt
        .screen()
        .lines
        .into_iter()
        .map(|line| line.text)
        .collect();
    let [status, approve, ..] = text.as_slice() else {
        panic!("screen has rows")
    };
    assert_eq!(
        status, "Status: working",
        "injected status text stays terminal data"
    );
    assert_eq!(approve, "Approve? [y/n]");

    let truncated = vt.feed(b"\x1b]7501;state=working");
    assert_eq!(
        truncated.osc,
        Vec::new(),
        "an unterminated OSC yields no record"
    );
    let open = vt.export_checkpoint();
    assert_eq!(open.pending_bytes(), b"\x1b]7501;state=working");
    assert_eq!(open.fidelity(), &Fidelity::Exact);

    let mut split = engine();
    split.feed(b"\xe4\xb8");
    assert_eq!(split.export_checkpoint().pending_bytes(), b"\xe4\xb8");

    let encoded = open.to_bytes().expect("checkpoint encodes");
    let mut stale: serde_json::Value =
        serde_json::from_slice(&encoded).expect("checkpoint is JSON");
    *stale.pointer_mut("/version").expect("version field") = serde_json::json!(0);
    let stale_bytes = serde_json::to_vec(&stale).expect("stale checkpoint encodes");
    let rejected = Checkpoint::from_bytes(&stale_bytes).expect_err("version 0 must be rejected");
    assert_eq!(rejected.kind(), "checkpoint_incompatible");

    let mut foreign = stale;
    *foreign.pointer_mut("/version").expect("version field") = serde_json::json!(1);
    *foreign.pointer_mut("/engine").expect("engine field") =
        serde_json::json!("another-emulator 9.9");
    let foreign_bytes = serde_json::to_vec(&foreign).expect("foreign checkpoint encodes");
    let foreign_error =
        Checkpoint::from_bytes(&foreign_bytes).expect_err("foreign engine rejected");
    assert_eq!(foreign_error.kind(), "checkpoint_incompatible");

    let garbage = Checkpoint::from_bytes(b"{\"format\":").expect_err("truncated JSON rejected");
    assert_eq!(garbage.kind(), "checkpoint_malformed");

    let summary = serde_json::json!({
        "osc_records": outcome.osc,
        "screen_lines": [status, approve],
        "truncated_osc_pending": String::from_utf8_lossy(open.pending_bytes()),
        "stale_version": {"kind": rejected.kind(), "message": rejected.to_string(), "hint": rejected.hint()},
        "foreign_engine": {"kind": foreign_error.kind(), "message": foreign_error.to_string()},
        "truncated_json": {"kind": garbage.kind(), "message": garbage.to_string()},
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).expect("summary serializes")
    );
}
