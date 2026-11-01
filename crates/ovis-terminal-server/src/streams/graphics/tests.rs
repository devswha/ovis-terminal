use ovis_terminal_protocol::domain::graphics::GraphicsCaps;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::ByteCount;
use ovis_terminal_vt::OutputOffset;
use proptest::prelude::*;
use sha2::{Digest, Sha256};

use super::super::filter::{QueryFilter, RawSlice};
use super::{Gated, GraphicsGate};

mod config;
mod dropped;

pub(super) const PNG: &[u8] = b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
pub(super) const SIXEL: &[u8] = b"\x1bP0;1;0q\"1;1;6;6#0;2;100;0;0#0~~~~~~-\x1b\\";

pub(super) fn kitty_series() -> Vec<u8> {
    let (first, rest) = PNG.split_at(32);
    let (second, third) = rest.split_at(32);
    [
        b"\x1b_Ga=T,f=100,i=7,m=1;".as_slice(),
        first,
        b"\x1b\\\x1b_Gm=1;",
        second,
        b"\x1b\\\x1b_Gm=0;",
        third,
        b"\x1b\\",
    ]
    .concat()
}

pub(super) fn iterm2() -> Vec<u8> {
    [b"\x1b]1337;File=inline=1:".as_slice(), PNG, b"\x07"].concat()
}

pub(super) fn caps(max: u64) -> GraphicsCaps {
    let mut caps = GraphicsCaps::advertised(&Limits::v1());
    caps.sequence_max_bytes = ByteCount::new(max);
    caps
}

pub(super) fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn width(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// What the hub does with `stream` read in pieces cut at `cuts`, then ended.
pub(super) fn run(max: u64, stream: &[u8], cuts: &[usize]) -> Vec<Gated> {
    run_answered(max, stream, (cuts, &[]))
}

pub(super) fn run_answered(
    max: u64,
    stream: &[u8],
    (cuts, answered): (&[usize], &[OutputOffset]),
) -> Vec<Gated> {
    let mut filter = QueryFilter::default();
    let mut gate = GraphicsGate::new(caps(max));
    let mut bounds: Vec<usize> = cuts
        .iter()
        .map(|cut| cut.checked_rem(stream.len().saturating_add(1)).unwrap_or(0))
        .chain([stream.len()])
        .collect();
    bounds.sort_unstable();
    let mut outputs = Vec::new();
    let mut from = 0;
    for to in bounds {
        let Some(bytes) = stream.get(from..to) else {
            continue;
        };
        let at = OutputOffset::new(width(from));
        outputs.push(gate.apply(&filter.apply(RawSlice {
            at,
            bytes,
            answered,
        })));
        from = to;
    }
    outputs.push(gate.apply(&filter.flush()));
    outputs.push(gate.flush());
    outputs
}

pub(super) fn released(outputs: &[Gated]) -> Vec<u8> {
    outputs
        .iter()
        .flat_map(|out| out.filtered.bytes.iter().copied())
        .collect()
}

/// Every output accounts for exactly the raw span after the previous one.
pub(super) fn accounted(outputs: &[Gated], total: usize) -> bool {
    let mut at = 0;
    for out in outputs {
        let filtered = &out.filtered;
        let removed: u64 = filtered.removed.iter().map(|(_, len)| *len).sum();
        let span = width(filtered.bytes.len()).saturating_add(removed);
        if filtered.raw_start.get() != at
            || filtered.raw_end.get() != filtered.raw_start.get().saturating_add(span)
        {
            return false;
        }
        at = filtered.raw_end.get();
    }
    at == width(total)
}

/// The sequence lies whole inside one output's bytes.
pub(super) fn released_whole(outputs: &[Gated], sequence: &[u8]) -> bool {
    outputs.iter().any(|out| {
        out.filtered
            .bytes
            .windows(sequence.len())
            .any(|window| window == sequence)
    })
}

fn framed(sequence: &[u8]) -> Vec<u8> {
    [b"text \x1b[1m".as_slice(), sequence, b"\x1b[0m more\r\n"].concat()
}

fn whole_at_every_split(sequence: &[u8]) {
    let stream = framed(sequence);
    for cut in 0..=stream.len() {
        let outputs = run(8 * 1024 * 1024, &stream, &[cut]);
        assert_eq!(sha(&released(&outputs)), sha(&stream), "cut {cut}");
        assert!(released_whole(&outputs, sequence), "cut {cut}");
        assert!(accounted(&outputs, stream.len()), "cut {cut}");
        assert!(outputs.iter().all(|out| out.dropped.is_empty()));
        let graphics: u64 = outputs.iter().map(|out| out.graphics).sum();
        assert_eq!(graphics, width(sequence.len()), "cut {cut}");
    }
}

#[test]
fn a_kitty_chunk_series_is_released_whole_at_every_split() {
    whole_at_every_split(&kitty_series());
}

#[test]
fn an_iterm2_image_is_released_whole_at_every_split() {
    whole_at_every_split(&iterm2());
}

#[test]
fn a_sixel_is_released_whole_at_every_split() {
    whole_at_every_split(SIXEL);
}

#[test]
fn other_strings_and_queries_pass_through_untouched() {
    let decrqss = b"\x1bP$qm\x1b\\";
    let title = b"\x1b]0;1337;File=title\x07";
    let apc = b"\x1b_Xnot graphics\x1b\\";
    let stream = [decrqss.as_slice(), title, apc, b"\x1b7\x1b[6n"].concat();
    let outputs = run(8, &stream, &[2, 9, 30]);
    assert_eq!(released(&outputs), stream);
    assert!(outputs.iter().all(|out| out.graphics == 0));
    assert!(outputs.iter().all(|out| out.dropped.is_empty()));
}

#[test]
fn an_answered_query_beside_an_image_is_still_removed() {
    let stream = [b"\x1b[6n".as_slice(), &kitty_series(), b"\x1b[6n"].concat();
    let answered = [OutputOffset::new(4), OutputOffset::new(width(stream.len()))];
    let outputs = run_answered(1024, &stream, (&[3, 7, 50], &answered));
    assert_eq!(released(&outputs), kitty_series());
    assert!(accounted(&outputs, stream.len()));
}

#[test]
fn text_between_chunks_ends_the_series_without_reordering() {
    let stream = [
        b"\x1b_Ga=T,m=1;AAAA\x1b\\".as_slice(),
        b"x",
        b"\x1b_Gm=0;BB\x1b\\",
    ]
    .concat();
    let outputs = run(1024, &stream, &[17]);
    assert_eq!(released(&outputs), stream);
    let graphics: u64 = outputs.iter().map(|out| out.graphics).sum();
    assert_eq!(graphics, width(stream.len().saturating_sub(1)));
}

#[test]
fn a_unit_still_arriving_stays_held_across_a_fence() {
    let series = kitty_series();
    let (first, rest) = series.split_at(40);
    let mut filter = QueryFilter::default();
    let mut gate = GraphicsGate::new(caps(1024));
    let slice = |at: usize, bytes| RawSlice {
        at: OutputOffset::new(width(at)),
        bytes,
        answered: &[],
    };
    let before = gate.apply(&filter.apply(slice(0, first)));
    let fenced = gate.apply(&filter.flush());
    let after = gate.apply(&filter.apply(slice(40, rest)));
    assert!(before.filtered.bytes.is_empty() && fenced.filtered.bytes.is_empty());
    assert_eq!(fenced.filtered.raw_end, OutputOffset::new(0));
    assert_eq!(after.filtered.bytes, series);
}

proptest! {
    #[test]
    fn every_byte_arrives_in_order_wherever_reads_split(
        text in proptest::collection::vec(0x20u8..0x7f, 0..24),
        pick in 0usize..3,
        cuts in proptest::collection::vec(0usize..400, 0..8),
    ) {
        let sequence = match pick { 0 => kitty_series(), 1 => iterm2(), _ => SIXEL.to_vec() };
        let stream = [text.as_slice(), &sequence, &text, &sequence].concat();
        let outputs = run(8 * 1024 * 1024, &stream, &cuts);
        prop_assert_eq!(sha(&released(&outputs)), sha(&stream));
        prop_assert!(released_whole(&outputs, &sequence));
        prop_assert!(accounted(&outputs, stream.len()));
    }

    #[test]
    fn arbitrary_output_is_never_lost_or_reordered(
        stream in proptest::collection::vec(any::<u8>(), 0..400),
        cuts in proptest::collection::vec(0usize..500, 0..8),
    ) {
        let outputs = run(8 * 1024 * 1024, &stream, &cuts);
        prop_assert!(accounted(&outputs, stream.len()));
        prop_assert_eq!(released(&outputs), stream);
    }
}
