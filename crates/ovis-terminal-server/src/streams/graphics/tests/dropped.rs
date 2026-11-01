use ovis_terminal_protocol::domain::graphics::{
    GraphicsDropReason, GraphicsDroppedData, GraphicsProtocol,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use proptest::prelude::*;

use super::super::Gated;
use super::{SIXEL, accounted, iterm2, kitty_series, released, run};

fn notices(outputs: &[Gated]) -> Vec<&GraphicsDroppedData> {
    outputs
        .iter()
        .flat_map(|out| out.dropped.iter().map(|(_, notice)| notice))
        .collect()
}

fn kitty_of(payload: usize) -> Vec<u8> {
    [
        b"\x1b_Ga=T,f=100;".as_slice(),
        &vec![b'A'; payload],
        b"\x1b\\",
    ]
    .concat()
}

/// `before`, then `sequence` dropped, then `after`: exactly one notice in its place.
fn dropped_whole(stream: (&[u8], &[u8], &[u8]), cuts: &[usize], max: u64) -> GraphicsDroppedData {
    let (before, sequence, after) = stream;
    let raw = [before, sequence, after].concat();
    let outputs = run(max, &raw, cuts);
    let delivered = released(&outputs);
    assert_eq!(delivered, [before, after].concat());
    assert!(accounted(&outputs, raw.len()));
    assert_eq!(outputs.iter().map(|out| out.graphics).sum::<u64>(), 0);
    let found = notices(&outputs);
    assert_eq!(found.len(), 1, "{found:?}");
    let index = outputs
        .iter()
        .find_map(|out| out.dropped.first().map(|(index, _)| *index))
        .unwrap_or(usize::MAX);
    let preceding: usize = outputs
        .iter()
        .take_while(|out| out.dropped.is_empty())
        .map(|out| out.filtered.bytes.len())
        .sum();
    assert_eq!(
        preceding.saturating_add(index),
        before.len(),
        "stands in place"
    );
    found.first().copied().cloned().expect("one notice")
}

fn expect(notice: &GraphicsDroppedData, protocol: GraphicsProtocol, (max, at): (u64, u64)) {
    assert_eq!(notice.protocol, OpenEnum::Known(protocol));
    assert_eq!(notice.reason, OpenEnum::Known(GraphicsDropReason::TooLarge));
    assert_eq!(notice.limit_bytes.get(), max);
    assert_eq!(notice.sequence_bytes.get(), max.saturating_add(1));
    assert_eq!(notice.output_offset.get(), at);
}

#[test]
fn the_cap_is_admitted_and_one_byte_more_is_dropped_whole() {
    let image = kitty_of(100);
    let max = u64::try_from(image.len()).unwrap_or(u64::MAX);
    let fits = run(max, &image, &[10]);
    assert_eq!(released(&fits), image);
    assert_eq!(notices(&fits), Vec::<&GraphicsDroppedData>::new());
    let notice = dropped_whole((b"ab", &image, b"cd"), &[10], max.saturating_sub(1));
    expect(&notice, GraphicsProtocol::Kitty, (max.saturating_sub(1), 2));
}

#[test]
fn every_protocol_over_its_cap_is_dropped_whole() {
    let cases = [
        (kitty_series(), GraphicsProtocol::Kitty),
        (iterm2(), GraphicsProtocol::Iterm2),
        (SIXEL.to_vec(), GraphicsProtocol::Sixel),
    ];
    for (sequence, protocol) in cases {
        let max = u64::try_from(sequence.len().saturating_sub(5)).unwrap_or(0);
        let notice = dropped_whole((b"x\x1b[1m", &sequence, b"y"), &[], max);
        expect(&notice, protocol, (max, 5));
    }
}

#[test]
fn chunks_after_the_drop_belong_to_the_dropped_series() {
    let series = kitty_series();
    let notice = dropped_whole((b"", &series, b"tail"), &[30, 60], 40);
    expect(&notice, GraphicsProtocol::Kitty, (40, 0));
}

#[test]
fn a_dropped_series_never_holds_more_than_its_cap() {
    let image = kitty_of(4096);
    let mut gate = super::super::GraphicsGate::new(super::caps(64));
    let mut filter = super::super::super::filter::QueryFilter::default();
    for (index, piece) in image.chunks(100).enumerate() {
        let at = ovis_terminal_vt::OutputOffset::new(
            u64::try_from(index.saturating_mul(100)).unwrap_or(u64::MAX),
        );
        let out = gate.apply(&filter.apply(super::super::super::filter::RawSlice {
            at,
            bytes: piece,
            answered: &[],
        }));
        assert_eq!(out.filtered.bytes, Vec::<u8>::new());
        assert_eq!(gate.held_start(), None, "nothing of a dropped unit is held");
    }
}

proptest! {
    #[test]
    fn an_over_cap_sequence_is_dropped_whole_wherever_reads_split(
        cuts in proptest::collection::vec(0usize..300, 0..8),
        pick in 0usize..3,
    ) {
        let sequence = match pick { 0 => kitty_series(), 1 => iterm2(), _ => SIXEL.to_vec() };
        let max = u64::try_from(sequence.len().saturating_sub(1)).unwrap_or(0);
        let notice = dropped_whole((b"left ", &sequence, b" right"), &cuts, max);
        prop_assert_eq!(notice.sequence_bytes.get(), max.saturating_add(1));
    }
}
