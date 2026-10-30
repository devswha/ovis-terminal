//! OSC bounds hold on the raw bytes vte buffers, whatever C0 controls the
//! program interleaves: controls that vte executes or ignores inside an
//! escape introducer or an OSC payload never change which record is emitted.

use ovis_terminal_vt::{
    Geometry, HistoryLimit, OscContent, OscRecord, OscTerminator, Vt, VtConfig,
};
use proptest::prelude::*;

#[cfg(test)]
fn engine() -> Vt {
    Vt::new(VtConfig::new(
        Geometry::new(20, 5).expect("geometry"),
        HistoryLimit::new(20),
    ))
}

/// C0 bytes vte executes or ignores without leaving an escape or OSC state.
#[cfg(test)]
const INERT_C0: [u8; 27] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x19, 0x1c, 0x1d, 0x1e,
];

#[cfg(test)]
fn contents(records: &[OscRecord]) -> Vec<(OscTerminator, OscContent)> {
    records
        .iter()
        .map(|record| (record.terminator, record.content.clone()))
        .collect()
}

#[test]
fn gate_osc_bounds_survive_control_inside_escape_introducer() {
    let mut vt = engine();
    let bytes = format!("\x1b\0]7501;{}{}\x07", "a;".repeat(15), "z".repeat(5000));
    let result = vt.feed(bytes.as_bytes());
    assert!(
        matches!(
            result.osc.first().map(|record| &record.content),
            Some(OscContent::Oversize { .. } | OscContent::Malformed { .. })
        ),
        "ignored C0 in Escape must not bypass raw OSC bounds: {:?}",
        result.osc
    );
}

#[test]
fn every_inert_control_in_the_introducer_keeps_the_bounds() {
    for control in INERT_C0.iter().copied().chain([0x1f, 0x7f]) {
        let mut vt = engine();
        let mut bytes = vec![0x1b, control];
        bytes.extend_from_slice(
            format!("]7501;{}{}\x07", "a;".repeat(15), "z".repeat(5000)).as_bytes(),
        );
        let records = vt.feed(&bytes).osc;
        let expected = u32::try_from(bytes.len() - 4).expect("fits");
        assert_eq!(
            records.first().map(|record| &record.content),
            Some(&OscContent::Oversize {
                number: Some(7501),
                bytes: expected
            }),
            "control {control:#04x}"
        );
    }
}

#[test]
fn ten_mib_osc_with_interleaved_controls_is_still_cut_at_the_bound() {
    let mut vt = engine();
    vt.feed(b"\x1b\x0e]7501;state=");
    let chunk: Vec<u8> = (0..64 * 1024)
        .map(|index| if index % 101 == 0 { 0 } else { b'z' })
        .collect();
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
    assert_eq!(vt.export_checkpoint().pending_bytes(), b"");
    vt.feed(b"\x07ok");
    assert_eq!(
        vt.screen().lines.first().map(|line| line.text.as_str()),
        Some("ok")
    );
}

#[cfg(test)]
fn osc() -> impl Strategy<Value = Vec<u8>> {
    (prop::collection::vec(0_usize..400, 1..25), any::<bool>()).prop_map(|(fields, bell)| {
        let mut bytes = b"\x1b]7501".to_vec();
        for (index, len) in fields.into_iter().enumerate() {
            bytes.push(b';');
            bytes.extend(std::iter::repeat_n(
                b'a'.saturating_add(u8::try_from(index % 26).unwrap_or(0)),
                len,
            ));
        }
        bytes.extend_from_slice(if bell { b"\x07" } else { b"\x1b\\" });
        bytes
    })
}

proptest! {
    #[test]
    fn interleaved_controls_never_change_the_records(
        sequences in prop::collection::vec(osc(), 1..4),
        injections in prop::collection::vec((any::<prop::sample::Index>(), prop::sample::select(INERT_C0.to_vec())), 0..30),
    ) {
        let clean = sequences.concat();
        let mut noisy = Vec::new();
        for sequence in &sequences {
            let mut sequence = sequence.clone();
            for (position, control) in &injections {
                // Inside the introducer (after ESC) or the payload, never
                // inside the terminator.
                let last = sequence.len().saturating_sub(if sequence.ends_with(b"\x07") { 1 } else { 2 });
                let at = position.index(last.saturating_sub(1)).saturating_add(1);
                sequence.insert(at, *control);
            }
            noisy.extend(sequence);
        }
        let expected = contents(&engine().feed(&clean).osc);
        prop_assert_eq!(contents(&engine().feed(&noisy).osc), expected);
    }
}
