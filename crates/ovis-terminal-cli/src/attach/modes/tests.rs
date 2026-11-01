//! Mode tracking under every chunk split, and the reset each mode leaves behind.

use proptest::prelude::*;

use super::ModeTracker;

const PROGRAM: &[u8] = b"\x1b[?2026h\x1b[2J\x1b[?1049h\x1b[?1h\x1b=\x1b[?1000;1006h\x1b[?2004h\
\x1b[?25l\x1b[>1u\x1b[>3u\x1b[<u\x1b[?2026l\x1b[38;5;196mred\x1b[?1000l";

fn tracked(chunks: &[&[u8]]) -> ModeTracker {
    let mut tracker = ModeTracker::default();
    for chunk in chunks {
        tracker.track(chunk);
    }
    tracker
}

#[test]
fn modes_left_on_are_reset_with_the_alternate_screen_last() {
    let reset = String::from_utf8(tracked(&[PROGRAM]).reset()).unwrap_or_default();
    assert_eq!(
        reset,
        "\x1b[?1l\x1b[?1006l\x1b[?2004l\x1b[?1049l\x1b[<1u\x1b>\x1b[?25h\x1b[0m"
    );
}

#[test]
fn nothing_drawn_resets_nothing_and_plain_text_only_resets_attributes() {
    assert_eq!(tracked(&[]).reset(), Vec::<u8>::new());
    assert_eq!(tracked(&[b"hello\r\n"]).reset(), b"\x1b[0m".to_vec());
}

#[test]
fn full_reset_and_cancelled_sequences_clear_or_skip_state() {
    assert_eq!(tracked(&[b"\x1b[?1049h\x1bc"]).reset(), b"\x1b[0m".to_vec());
    assert_eq!(tracked(&[b"\x1b[?20\x1804h"]).reset(), b"\x1b[0m".to_vec());
    assert_eq!(
        tracked(&[b"\x1b[?99999h\x1b[?70000;2004h"]).reset(),
        b"\x1b[0m".to_vec(),
        "an overflowing parameter applies nothing"
    );
}

proptest! {
    #[test]
    fn any_split_of_the_stream_tracks_the_same_modes(
        cuts in proptest::collection::vec(0..PROGRAM.len(), 0..12)
    ) {
        let mut cuts = cuts;
        cuts.sort_unstable();
        let mut chunks = Vec::new();
        let mut from = 0;
        for cut in cuts {
            chunks.push(PROGRAM.get(from..cut).unwrap_or_default());
            from = cut;
        }
        chunks.push(PROGRAM.get(from..).unwrap_or_default());
        prop_assert_eq!(tracked(&chunks).reset(), tracked(&[PROGRAM]).reset());
    }

    #[test]
    fn arbitrary_bytes_never_panic_and_always_end_with_an_attribute_reset(
        bytes in proptest::collection::vec(any::<u8>(), 1..512)
    ) {
        let reset = tracked(&[&bytes]).reset();
        prop_assert!(reset.ends_with(b"\x1b[0m"));
    }
}
