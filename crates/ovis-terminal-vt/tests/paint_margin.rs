//! DECAWM-off margin state must survive late attach and subsequent bytes.

use ovis_terminal_vt::{Fidelity, Geometry, HistoryLimit, Vt, VtConfig};

#[test]
fn decawm_off_wide_char_after_tab_matches_live_at_every_split() {
    // Given the minimized task-12 stream on its original 10-column grid.
    let config = VtConfig::new(
        Geometry::new(10, 6).expect("geometry"),
        HistoryLimit::new(100),
    );
    let input = "\x1b[?7laa aaa  a\t\u{4e2d}\u{6587}".as_bytes();
    for split in 0..=input.len() {
        let (before, after) = input.split_at(split);
        let mut live = Vt::new(config);
        live.feed(before);
        let checkpoint = live.export_checkpoint();
        let mut restored = Vt::from_checkpoint(&checkpoint).expect("checkpoint");
        let paint = restored.first_paint();
        assert_eq!(paint.fidelity, Fidelity::Exact, "split {split}");
        let mut attached = Vt::new(config);

        // When a checkpoint's first paint precedes the remaining live bytes.
        attached.feed(&paint.bytes);
        live.feed(after);
        restored.feed(after);
        attached.feed(after);

        // Then both restoration routes reproduce the live screen and cursor.
        assert_eq!(
            restored.screen().lines,
            live.screen().lines,
            "checkpoint split {split}"
        );
        assert_eq!(
            attached.screen().lines,
            live.screen().lines,
            "paint split {split}"
        );
        assert_eq!(attached.cursor(), live.cursor(), "cursor split {split}");
        assert_eq!(attached.modes(), live.modes(), "modes split {split}");
    }
    println!("live vs restored screens equal at every byte split, including the right-margin tab");
}
