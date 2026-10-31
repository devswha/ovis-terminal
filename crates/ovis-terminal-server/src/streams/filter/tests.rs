use ovis_terminal_vt::OutputOffset;
use proptest::prelude::*;

use super::{QueryFilter, RawSlice};

const DSR: &[u8] = b"\x1b[6n";
const OSC_QUERY: &[u8] = b"\x1b]10;?\x1b\\";

/// Filters `stream` cut at `cuts`, then flushes, returning released bytes.
fn run(stream: &[u8], cuts: &[usize], answered: &[OutputOffset]) -> Vec<u8> {
    let mut filter = QueryFilter::default();
    let mut released = Vec::new();
    let mut from = 0;
    let mut bounds: Vec<usize> = cuts
        .iter()
        .map(|cut| cut.checked_rem(stream.len().saturating_add(1)).unwrap_or(0))
        .collect();
    bounds.push(stream.len());
    bounds.sort_unstable();
    for to in bounds {
        let Some(bytes) = stream.get(from..to) else {
            continue;
        };
        let out = filter.apply(RawSlice {
            at: OutputOffset::new(u64::try_from(from).unwrap_or(u64::MAX)),
            bytes,
            answered,
        });
        released.extend(out.bytes);
        from = to;
    }
    released.extend(filter.flush().bytes);
    released
}

fn end_of(stream: &[u8], needle: &[u8]) -> OutputOffset {
    let start = stream
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or(stream.len());
    OutputOffset::new(u64::try_from(start.saturating_add(needle.len())).unwrap_or(u64::MAX))
}

#[test]
fn an_answered_query_split_across_reads_is_removed_whole_and_mapped() {
    let stream = [b"ab".as_slice(), DSR, b"cd"].concat();
    let answered = [end_of(&stream, DSR)];
    let mut filter = QueryFilter::default();
    let first = filter.apply(RawSlice {
        at: OutputOffset::new(0),
        bytes: stream.get(..4).unwrap_or_default(),
        answered: &answered,
    });
    assert_eq!(first.bytes, b"ab");
    assert_eq!(
        first.raw_end,
        OutputOffset::new(2),
        "the open query is held"
    );
    let second = filter.apply(RawSlice {
        at: OutputOffset::new(4),
        bytes: stream.get(4..).unwrap_or_default(),
        answered: &answered,
    });
    assert_eq!(second.bytes, b"cd");
    assert_eq!(second.raw_start, OutputOffset::new(2));
    assert_eq!(second.raw_end, OutputOffset::new(8));
    assert_eq!(second.removed, vec![(0, 4)]);
}

#[test]
fn an_unanswered_query_and_ordinary_sequences_pass_through() {
    let stream = [b"\x1b[31mred".as_slice(), DSR, b"\x1b]0;title\x07"].concat();
    assert_eq!(run(&stream, &[3, 9], &[]), stream);
}

#[test]
fn refusal_like_application_text_remains_terminal_data() {
    let stream = b"herdr: server shut down: attach refused\r\n".to_vec();
    assert_eq!(run(&stream, &[5], &[]), stream);
}

proptest! {
    #[test]
    fn removal_does_not_depend_on_read_boundaries(
        prefix in proptest::collection::vec(any::<u8>().prop_filter("no ESC", |b| *b != 0x1b && *b != 0x18 && *b != 0x1a), 0..40),
        middle in proptest::collection::vec(0x20u8..0x7f, 0..40),
        cuts in proptest::collection::vec(0usize..200, 0..6),
    ) {
        let stream = [prefix.as_slice(), DSR, middle.as_slice(), OSC_QUERY, b"z"].concat();
        let answered = [end_of(&stream, DSR), end_of(&stream, OSC_QUERY)];
        let expected = [prefix.as_slice(), middle.as_slice(), b"z"].concat();
        prop_assert_eq!(run(&stream, &cuts, &answered), expected);
    }

    #[test]
    fn without_answered_queries_every_byte_is_released_in_order(
        stream in proptest::collection::vec(any::<u8>(), 0..300),
        cuts in proptest::collection::vec(0usize..400, 0..8),
    ) {
        prop_assert_eq!(run(&stream, &cuts, &[]), stream);
    }
}
