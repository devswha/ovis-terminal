//! The units the gate reports to the image store, and the paint that re-sends them.

use std::sync::Arc;

use ovis_terminal_core::terminals::images::{ImageStore, ReleasedUnit};
use ovis_terminal_protocol::domain::graphics::{GraphicsCaps, GraphicsProtocol};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, HistoryLimit, OutputOffset, Vt, VtConfig};

use super::super::Gated;
use super::super::tests::{SIXEL, run};
use super::painted;

fn units(outputs: &[Gated]) -> Vec<(u64, usize)> {
    outputs
        .iter()
        .flat_map(|out| out.units.iter())
        .map(|(_, unit)| (unit.key.get(), unit.bytes.len()))
        .collect()
}

fn dropped(outputs: &[Gated]) -> Vec<(u64, u64)> {
    outputs
        .iter()
        .flat_map(|out| out.dropped.iter())
        .map(|(_, data)| (data.output_offset.get(), data.sequence_bytes.get()))
        .collect()
}

fn chunk(keys: &str, payload: usize) -> Vec<u8> {
    [
        format!("\x1b_G{keys};").as_bytes(),
        &vec![b'A'; payload],
        b"\x1b\\",
    ]
    .concat()
}

#[test]
fn an_interrupted_kitty_series_is_one_key_measured_against_the_cap_together() {
    let (first, last) = (chunk("a=T,i=1,m=1", 60), chunk("m=0", 60));
    let stream = [first.as_slice(), b"text", &last].concat();
    let total = u64::try_from(first.len() + last.len()).expect("small");
    let kept = run(total, &stream, &[]);
    let first_len = first.len();
    assert_eq!(units(&kept), [(0, first_len), (0, last.len())]);
    assert_eq!(dropped(&kept), Vec::<(u64, u64)>::new());
    let over = run(total - 1, &stream, &[]);
    assert_eq!(
        units(&over),
        [(0, first_len)],
        "the run that passes the cap is dropped"
    );
    assert_eq!(
        dropped(&over),
        [(0, total)],
        "one notice at the series start"
    );
}

#[test]
fn an_osc_1337_that_is_not_an_inline_file_is_not_graphics() {
    let outputs = run(1 << 20, b"\x1b]1337;SetUserVar=a=Yg==\x07", &[]);
    assert_eq!(units(&outputs), Vec::<(u64, usize)>::new());
}

#[test]
fn kitty_keys_are_read_up_to_256_bytes_for_the_continuation_flag() {
    let keys = |pad: usize| format!("a=T,i=1,{}m=1", "q=0,".repeat(pad));
    let series = |pad: usize| [chunk(&keys(pad), 8), chunk("m=0", 8)].concat();
    assert_eq!(
        units(&run(1 << 20, &series(25), &[])).len(),
        1,
        "111 key bytes"
    );
    assert_eq!(
        units(&run(1 << 20, &series(61), &[])).len(),
        1,
        "255 key bytes"
    );
    assert_eq!(
        units(&run(1 << 20, &series(62), &[])).len(),
        2,
        "259 key bytes"
    );
}

#[test]
fn sixel_parameters_are_read_up_to_63_bytes() {
    let sixel = |params: usize| [b"\x1bP".as_slice(), &vec![b'1'; params], b"q#0~-\x1b\\"].concat();
    assert_eq!(units(&run(1 << 20, &sixel(20), &[])).len(), 1);
    assert_eq!(units(&run(1 << 20, &sixel(63), &[])).len(), 1);
    assert_eq!(units(&run(1 << 20, &sixel(64), &[])).len(), 0);
}

#[test]
fn a_paint_shares_the_store_bytes_where_its_images_go() {
    let geometry = Geometry::new(20, 4).expect("valid geometry");
    let mut vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(10)));
    drop(vt.feed(b"top\r\n"));
    drop(vt.feed(SIXEL));
    drop(vt.feed(b"\r\n"));
    drop(vt.feed(&chunk("a=T,i=2", 4)));
    let shown = vt.images();
    let mut store = ImageStore::new(GraphicsCaps::advertised(&Limits::v1()));
    let unit = ReleasedUnit::new(
        OutputOffset::new(5),
        GraphicsProtocol::Sixel,
        Arc::from(SIXEL),
    );
    assert_eq!(store.admit(unit, &shown), None);
    let paint = vt.first_paint();
    let at = paint
        .images
        .first()
        .map(|splice| splice.at)
        .expect("spliced");
    let expected = [
        paint.bytes.get(..at).unwrap_or_default(),
        b"\x1b[2;1H",
        SIXEL,
        paint.bytes.get(at..).unwrap_or_default(),
    ]
    .concat();
    let sent = painted(paint, &store);
    let bytes: Vec<u8> = sent
        .pieces
        .iter()
        .flat_map(|piece| piece.bytes.as_slice().to_vec())
        .collect();
    assert_eq!(
        bytes, expected,
        "the Kitty image the store lacks is skipped"
    );
    let replay: Vec<bool> = sent.pieces.iter().map(|piece| piece.replay).collect();
    assert_eq!(replay, [false, false, true, false]);
    let data = sent.replay.expect("a replay record");
    assert_eq!(
        data.replayed.iter().map(|at| at.get()).collect::<Vec<_>>(),
        [5]
    );
}
