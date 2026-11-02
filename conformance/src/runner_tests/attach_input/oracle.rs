//! Screens rebuilt the way a client would (checkpoint paint plus the stream bytes after it)
//! against fresh VTs fed the uninterrupted raw output.

use base64::Engine as _;
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use serde_json::{Value, json};

use super::super::support::{Ran, TestResult};
use super::{decimal, held};

/// Bytes the scenario's `head -c 300000 | tr '\000' y` flood writes to the overflowed t1.
const FLOOD: usize = 300_000;

/// The visible lines and the `{"row", "col", "visible"}` cursor of a VT.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Screen {
    pub(super) lines: Vec<String>,
    pub(super) cursor: Value,
}

/// A fresh 80x24 VT fed `bytes` in one piece, whatever chunks they arrived in.
pub(super) fn screen(bytes: &[u8]) -> TestResult<Screen> {
    let mut vt = Vt::new(VtConfig::new(
        Geometry::new(80, 24)?,
        HistoryLimit::new(10_000),
    ));
    vt.feed(bytes);
    let lines = vt
        .screen()
        .lines
        .into_iter()
        .map(|line| line.text)
        .collect();
    let cursor = vt.cursor();
    let cursor = json!({"row": cursor.row, "col": cursor.column, "visible": cursor.visible});
    Ok(Screen { lines, cursor })
}

/// The base64 bytes the binary answered for one named check.
pub(super) fn bytes(ran: &Ran, check: &str) -> TestResult<Vec<u8>> {
    let text = held(ran, check)?.as_str().ok_or("base64 bytes")?;
    Ok(base64::engine::general_purpose::STANDARD.decode(text)?)
}

fn position(ran: &Ran, check: &str) -> TestResult<usize> {
    Ok(usize::try_from(decimal(ran, check)?)?)
}

/// t2: the late observer's first paint equals a VT fed the raw output up to the paint's
/// offset, and its repaint plus the tail after it equals a VT fed the whole raw output.
/// Two more observers attach inside a split UTF-8 character (offset 6) and an open CSI
/// (offset 16); each one's paint plus the tail it was sent equals the whole raw output.
pub(super) fn late_paint_and_tail_rebuild_the_screen(ran: &Ran) -> TestResult {
    let raw = bytes(ran, "reply[fx-raw]/result/data_b64")?;
    let stream = bytes(ran, "stream[t2]/data_b64")?;
    let repaint_at = position(ran, "reply[late-repaint]/result/stream_offset")?;
    let painted_at = position(ran, "reply[late]/result/first_paint/output_offset")?;
    let (first_paint, repaint_and_tail) = stream.split_at_checked(repaint_at).ok_or("repaint")?;
    let up_to_paint = raw.get(..painted_at).ok_or("paint offset")?;
    assert_eq!(screen(first_paint)?, screen(up_to_paint)?, "first paint");
    assert_eq!(
        screen(repaint_and_tail)?,
        screen(&raw)?,
        "repaint plus tail"
    );
    for (fence, painted_at) in [("utf8", 6), ("csi", 16)] {
        let at = position(
            ran,
            &format!("reply[split-{fence}]/result/first_paint/output_offset"),
        )?;
        assert_eq!(
            at, painted_at,
            "{fence}: the paint sits inside the partial sequence"
        );
        let stream = bytes(ran, &format!("stream[t2-{fence}]/data_b64"))?;
        assert_eq!(screen(&stream)?, screen(&raw)?, "{fence}: paint plus tail");
    }
    Ok(())
}

/// t1 after overflow: the observer's repaint plus the tail it was sent equals a VT fed the
/// uninterrupted output. Evicted bytes are the flood's known `y`s; the retained bytes after
/// them and the tail come from `terminal.output`.
pub(super) fn repaint_after_overflow_and_tail_rebuild_the_screen(ran: &Ran) -> TestResult {
    let retained = bytes(ran, "reply[floor-read]/result/data_b64")?;
    let tail = bytes(ran, "reply[tail-raw]/result/data_b64")?;
    let client = bytes(ran, "stream[t1]/data_b64")?;
    assert_eq!(
        decimal(ran, "reply[tail-raw]/result/from_offset")?,
        decimal(ran, "reply[screen]/result/output_offset")?,
        "the tail starts where the retained output ended"
    );
    let flood_end = retained
        .iter()
        .rposition(|byte| *byte == b'y')
        .ok_or("the retained output ends inside the flood")?;
    let (flood_tail, after_flood) = retained.split_at(flood_end.saturating_add(1));
    assert!(
        flood_tail.iter().all(|byte| *byte == b'y'),
        "retained flood"
    );
    let mut uninterrupted = b"y".repeat(FLOOD);
    uninterrupted.extend_from_slice(after_flood);
    uninterrupted.extend_from_slice(&tail);
    assert_eq!(screen(&client)?, screen(&uninterrupted)?);
    Ok(())
}
