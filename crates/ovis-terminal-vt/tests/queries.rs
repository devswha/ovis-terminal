//! Query replies are located in the stream, answered once, and never
//! re-answered when retained output is replayed into a restored engine.

use ovis_terminal_vt::{
    ClientFact, Geometry, HistoryLimit, OutputOffset, QueryOutcome, TerminalQuery, Vt, VtConfig,
};

#[cfg(test)]
fn engine(columns: u16, rows: u16) -> Vt {
    let geometry = Geometry::new(columns, rows).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(50)))
}

/// Offset just past the first occurrence of `query` in `stream`.
#[cfg(test)]
fn end_of(stream: &[u8], query: &[u8]) -> u64 {
    let start = stream
        .windows(query.len())
        .position(|window| window == query)
        .expect("query present");
    u64::try_from(start.checked_add(query.len()).expect("offset fits")).expect("offset fits")
}

const fn reply(query: &TerminalQuery) -> Option<(u64, &[u8])> {
    match &query.outcome {
        QueryOutcome::Reply(bytes) => Some((query.offset.get(), bytes.as_slice())),
        _ => None,
    }
}

#[test]
fn replies_carry_the_offset_of_their_query() {
    let mut vt = engine(20, 5);
    let stream: &[u8] = b"ab\x1b[6n\x1b[ccd\x1b[?9001h\x1b[?9001$p\x1b[>4;1m\x1b[?4m";
    let outcome = vt.feed(stream);
    let replies: Vec<_> = outcome.queries.iter().filter_map(reply).collect();
    assert_eq!(
        replies,
        vec![
            (end_of(stream, b"\x1b[6n"), b"\x1b[1;3R".as_slice()),
            (end_of(stream, b"\x1b[c"), b"\x1b[?6c".as_slice()),
            (
                end_of(stream, b"\x1b[?9001$p"),
                b"\x1b[?9001;1$y".as_slice()
            ),
            (end_of(stream, b"\x1b[?4m"), b"\x1b[>4;1m".as_slice()),
        ]
    );
}

#[test]
fn split_query_is_answered_once_at_its_final_byte() {
    let mut vt = engine(20, 5);
    let first = vt.feed(b"x\x1b[");
    let second = vt.feed(b"6n");
    assert_eq!(first.queries, Vec::new());
    assert_eq!(
        second.queries.iter().filter_map(reply).collect::<Vec<_>>(),
        vec![(end_of(b"x\x1b[6n", b"\x1b[6n"), b"\x1b[1;2R".as_slice())]
    );
}

#[test]
fn replayed_queries_are_not_answered_twice() {
    let mut live = engine(20, 5);
    live.feed(b"before ");
    let checkpoint = live.export_checkpoint();
    let retained: &[u8] = b"\x1b[6n middle \x1b[c after \x1b[5n";
    let (answered_part, _) = retained.split_at(20);
    let answered = live.feed(answered_part);
    assert_eq!(
        answered.queries.len(),
        2,
        "the replaced engine answered two queries"
    );

    let mut restored = Vt::from_checkpoint(&checkpoint).expect("imports");
    restored.assume_queries_answered_through(OutputOffset::new(checkpoint.offset().get() + 20));
    let replay = restored.feed(retained);
    assert_eq!(replay.suppressed_queries, 2);
    let status_end = checkpoint.offset().get() + end_of(retained, b"\x1b[5n");
    assert_eq!(
        replay.queries.iter().filter_map(reply).collect::<Vec<_>>(),
        vec![(status_end, b"\x1b[0n".as_slice())]
    );
}

#[test]
fn queries_needing_client_facts_are_not_invented() {
    let mut vt = engine(20, 5);
    let outcome = vt.feed(b"\x1b]11;?\x07\x1b[14t\x1b]4;1;rgb:aa/bb/cc\x07\x1b]4;1;?\x07");
    let outcomes: Vec<&QueryOutcome> = outcome.queries.iter().map(|query| &query.outcome).collect();
    assert_eq!(
        outcomes,
        vec![
            &QueryOutcome::NeedsClientFact(ClientFact::PaletteColor { index: 257 }),
            &QueryOutcome::NeedsClientFact(ClientFact::CellPixelSize),
            &QueryOutcome::Reply(b"\x1b]4;1;rgb:aaaa/bbbb/cccc\x07".to_vec()),
        ]
    );
}

#[test]
fn cursor_report_after_resize_uses_the_new_grid() {
    let mut vt = engine(20, 8);
    vt.feed(b"\x1b[8;20H");
    vt.resize(Geometry::new(10, 4).expect("valid geometry"));
    let outcome = vt.feed(b"\x1b[6n\x1b[18t");
    let replies: Vec<_> = outcome
        .queries
        .iter()
        .filter_map(reply)
        .map(|(_, bytes)| bytes.to_vec())
        .collect();
    assert_eq!(
        replies,
        vec![b"\x1b[4;10R".to_vec(), b"\x1b[8;4;10t".to_vec()]
    );
}
