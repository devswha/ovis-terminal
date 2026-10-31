//! Wire forms and chunking: counters travel as canonical decimal strings and
//! every split of the holder stream yields the same reads.

use ovis_terminal_core::screen::{
    HistoryRequest, OutputRequest, PageCursor, ScreenBounds, SearchRequest, decode_request,
};
use proptest::prelude::*;

use crate::fixture::{TestResult, limit, needle, nonzero, offset, output, revision, screen};

#[test]
fn u64_max_counters_round_trip_as_decimal_strings() -> TestResult {
    // Given: a cursor at the largest revision and an output request at the largest offset.
    let cursor = r#"{"cursor":{"revision":"18446744073709551615","line":3},"limit":9}"#;
    let raw = r#"{"from":"18446744073709551615","limit_bytes":5}"#;

    // When: both are decoded and encoded again.
    let page: HistoryRequest = decode_request(cursor.as_bytes()).expect("decimal cursor");
    let bytes: OutputRequest = decode_request(raw.as_bytes()).expect("decimal offset");

    // Then: the values are exact and re-encode to the same strings.
    assert_eq!(
        page,
        HistoryRequest::new(limit(9)).at(PageCursor::new(revision(u64::MAX)?, 3))
    );
    assert_eq!(bytes, OutputRequest::new(offset(u64::MAX), limit(5)));
    assert_eq!(serde_json::to_string(&page).expect("encode"), cursor);
    assert_eq!(serde_json::to_string(&bytes).expect("encode"), raw);
    Ok(())
}

#[test]
fn numeric_and_non_canonical_counters_are_invalid_params() {
    for json in [
        r#"{"revision":18446744073709551615,"needle":"x","limit":1}"#,
        r#"{"revision":"007","needle":"x","limit":1}"#,
        r#"{"revision":"-1","needle":"x","limit":1}"#,
        r#"{"revision":"1","needle":"x","limit":1,"focused":true}"#,
        r#"{"needle":"","limit":1}"#,
        r#"{"needle":"x","limit":0}"#,
    ] {
        let error = decode_request::<SearchRequest>(json.as_bytes()).expect_err(json);
        assert_eq!(error.kind(), "invalid_params", "{json}");
    }
}

#[test]
fn emitted_reads_carry_counters_as_strings() -> TestResult {
    // Given: a screen with output.
    let screen = screen("terminal:wire", ScreenBounds::DEFAULT)?;
    crate::fixture::feed(&screen, b"hello\r\n")?;

    // When: a view is encoded.
    let encoded = serde_json::to_value(screen.screen().expect("screen")).expect("encode");

    // Then: offsets and revisions are strings, never JSON numbers.
    for pointer in [
        "/meta/offset",
        "/meta/revision",
        "/meta/live/revision",
        "/meta/live/offset",
    ] {
        assert!(
            encoded
                .pointer(pointer)
                .is_some_and(serde_json::Value::is_string),
            "{pointer}"
        );
    }
    Ok(())
}

fn chunked(bytes: &[u8], cuts: &[usize]) -> Vec<Vec<u8>> {
    let mut points: Vec<usize> = cuts
        .iter()
        .filter_map(|cut| cut.checked_rem(bytes.len().saturating_add(1)))
        .collect();
    points.sort_unstable();
    points.dedup();
    let mut chunks = Vec::new();
    let mut from = 0;
    for point in points.into_iter().chain(std::iter::once(bytes.len())) {
        if let Some(chunk) = bytes.get(from..point).filter(|chunk| !chunk.is_empty()) {
            chunks.push(chunk.to_vec());
            from = point;
        }
    }
    chunks
}

fn stream() -> impl Strategy<Value = Vec<u8>> {
    let pieces = prop_oneof![
        "[a-z ]{1,12}".prop_map(String::into_bytes),
        Just(b"\r\n".to_vec()),
        Just("\u{e9}\u{4e2d}".as_bytes().to_vec()),
        Just(b"\x1b[1;31m".to_vec()),
        Just(b"\x1b[?1049h".to_vec()),
        Just(b"\x1b[?1049l".to_vec()),
        Just(b"\x1b[6n".to_vec()),
    ];
    prop::collection::vec(pieces, 1..60).prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn every_split_yields_identical_reads(bytes in stream(), cuts in prop::collection::vec(any::<usize>(), 0..12)) {
        // Given: the same stream ingested whole and in arbitrary chunks with a tiny work budget.
        let whole = screen("terminal:whole", ScreenBounds::DEFAULT).expect("whole screen");
        let split = screen("terminal:split", ScreenBounds::DEFAULT.with_work_budget(nonzero(3))).expect("split screen");
        whole.ingest(&output(whole.target(), 0, &bytes)).expect("whole");
        let mut at = 0;
        for chunk in chunked(&bytes, &cuts) {
            split.ingest(&output(split.target(), at, &chunk)).expect("chunk");
            at = at.saturating_add(u64::try_from(chunk.len()).expect("length"));
        }

        // When: both are read.
        let (left, right) = (whole.screen().expect("whole"), split.screen().expect("split"));
        let request = OutputRequest::new(offset(0), limit(u32::MAX));

        // Then: screen, revision, offset, raw bytes and query replies agree.
        prop_assert_eq!(&left.lines, &right.lines);
        prop_assert_eq!(left.meta.revision, right.meta.revision);
        prop_assert_eq!(left.meta.offset, right.meta.offset);
        prop_assert_eq!(&whole.output(&request).expect("whole").bytes, &bytes);
        prop_assert_eq!(&split.output(&request).expect("split").bytes, &bytes);
        prop_assert_eq!(whole.drain_replies().expect("whole"), split.drain_replies().expect("split"));
        let search = SearchRequest::new(needle("e").expect("needle"), limit(1000));
        prop_assert_eq!(
            whole.search(&search).expect("whole").matches,
            split.search(&search).expect("split").matches
        );
    }
}
