//! Chunk boundaries never change engine state, and a checkpoint taken at any
//! byte boundary either reproduces the live engine or says why it cannot.

use ovis_terminal_vt::{Checkpoint, Fidelity, Geometry, HistoryLimit, InexactReason, Vt, VtConfig};
use proptest::prelude::*;

#[cfg(test)]
fn engine() -> Vt {
    let geometry = Geometry::new(12, 5).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(20)))
}

fn token() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        "[a-z ]{1,8}".prop_map(String::into_bytes),
        Just("\u{4e2d}\u{1f600}e\u{301}".as_bytes().to_vec()),
        Just(b"\r\n\t".to_vec()),
        (1_u8..7, 1_u8..13).prop_map(|(row, column)| format!("\x1b[{row};{column}H").into_bytes()),
        (0_u8..108).prop_map(|code| format!("\x1b[{code}m").into_bytes()),
        prop::sample::select(vec![
            "1049", "2004", "1002", "1006", "9001", "2026", "6", "7", "1"
        ])
        .prop_flat_map(|mode| prop::bool::ANY.prop_map(move |on| {
            format!("\x1b[?{mode}{}", if on { 'h' } else { 'l' }).into_bytes()
        })),
        Just(b"\x1b[2;4r\x1b7".to_vec()),
        Just(b"\x1b8\x1b[K\x1b[2P\x1b[L".to_vec()),
        Just(b"\x1b[>5u\x1b[=3;2u\x1b[<u\x1b[>4;2m".to_vec()),
        Just(b"\x1b]2;title\x07\x1b[22t\x1b[23t".to_vec()),
        Just(b"\x1b]7501;state=idle\x1b\\".to_vec()),
        Just(b"\x1b(0q\x0e\x0f\x1b(B".to_vec()),
        Just(b"\x1bH\x1b[3g\x1b[g".to_vec()),
        Just(b"\x1b[5bX\x1b[3b".to_vec()),
        Just(b"\x1b[6n\x1b[c".to_vec()),
        Just(b"\x1b[1;\n2H".to_vec()),
        Just(b"\x1bc".to_vec()),
    ]
}

fn stream() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(token(), 0..40).prop_map(|tokens| tokens.concat())
}

proptest! {
    #[test]
    fn chunking_never_changes_state(bytes in stream(), size in 1_usize..9) {
        let mut whole = engine();
        whole.feed(&bytes);
        let mut chunked = engine();
        for chunk in bytes.chunks(size) {
            chunked.feed(chunk);
        }
        prop_assert_eq!(chunked.export_checkpoint(), whole.export_checkpoint());
    }

    #[test]
    fn checkpoint_at_any_boundary_is_exact_or_says_why(bytes in stream(), cut in any::<prop::sample::Index>()) {
        let split = cut.index(bytes.len().saturating_add(1));
        let (head, tail) = bytes.split_at(split);
        let mut live = engine();
        live.feed(head);
        let checkpoint = Checkpoint::from_bytes(&live.export_checkpoint().to_bytes().expect("encodes"))
            .expect("decodes");
        let mut restored = Vt::from_checkpoint(&checkpoint).expect("imports");
        live.feed(tail);
        restored.feed(tail);
        match checkpoint.fidelity() {
            Fidelity::Exact => prop_assert_eq!(restored.export_checkpoint(), live.export_checkpoint()),
            Fidelity::Inexact { reasons } => {
                prop_assert!(reasons.iter().all(|reason| matches!(
                    reason,
                    InexactReason::ControlInsideOpenSequence
                        | InexactReason::PrimarySavedCursorUnrepresentable
                )));
                prop_assert_eq!(restored.fidelity(), checkpoint.fidelity());
            },
            _ => prop_assert!(false, "unexpected fidelity {:?}", checkpoint.fidelity()),
        }
    }
}

#[test]
fn control_inside_an_open_sequence_is_reported_not_replayed() {
    let mut live = engine();
    live.feed(b"top\x1b[1;\n");
    let checkpoint = live.export_checkpoint();
    assert_eq!(
        checkpoint.fidelity(),
        &Fidelity::Inexact {
            reasons: vec![InexactReason::ControlInsideOpenSequence]
        }
    );
    assert_eq!(checkpoint.pending_bytes(), b"");
    let restored = Vt::from_checkpoint(&checkpoint).expect("imports");
    assert_eq!(restored.fidelity(), checkpoint.fidelity());
    assert_eq!(restored.screen().fidelity, *checkpoint.fidelity());
}
