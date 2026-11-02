//! Where a paint re-sends retained images, and that the re-sent paint rebuilds them.

use super::{SIXEL, engine, keys, kitty};
use crate::engine::Vt;
use crate::engine::images::ImagePlace;

/// The paint with each splice's cursor move and the image's own bytes inserted.
fn replayed(vt: &Vt, bytes_of: impl Fn(u64) -> Vec<u8>) -> Vec<u8> {
    let paint = vt.first_paint();
    let mut out = Vec::new();
    let mut from = 0;
    for splice in &paint.images {
        out.extend_from_slice(paint.bytes.get(from..splice.at).unwrap_or_default());
        out.extend_from_slice(&splice.cursor);
        out.extend(bytes_of(splice.key.get()));
        from = splice.at;
    }
    out.extend_from_slice(paint.bytes.get(from..).unwrap_or_default());
    out
}

fn places(vt: &Vt) -> Vec<Option<ImagePlace>> {
    vt.images()
        .retained
        .iter()
        .map(|image| image.place)
        .collect()
}

fn fed(chunks: &[&[u8]]) -> Vt {
    let mut vt = engine();
    for chunk in chunks {
        drop(vt.feed(chunk));
    }
    vt
}

#[test]
fn a_shown_image_is_spliced_after_the_rows_and_the_replay_rebuilds_it_in_place() {
    let image = kitty("a=T,i=1");
    let live = fed(&[b"top\r\n", &image, b"\r\nbelow"]);
    let paint = live.first_paint();
    let splices: Vec<(u64, &[u8])> = paint
        .images
        .iter()
        .map(|splice| (splice.key.get(), splice.cursor.as_slice()))
        .collect();
    assert_eq!(splices, [(5, b"\x1b[2;1H".as_slice())]);
    let fresh = fed(&[&replayed(&live, |_| image.clone())]);
    assert_eq!(fresh.screen().lines, live.screen().lines);
    assert_eq!(fresh.screen().cursor, live.screen().cursor);
    assert_eq!(places(&fresh), places(&live));
}

#[test]
fn a_put_follows_its_transmission_and_a_put_without_one_is_skipped() {
    let (data, put, orphan) = (kitty("a=t,i=1"), kitty("a=p,i=1"), kitty("a=p,i=9"));
    let live = fed(&[&data, b"\r\n", &put, b"\r\n", &orphan]);
    assert_eq!(keys(&live).len(), 3);
    let paint = live.first_paint();
    let splices: Vec<(u64, &[u8])> = paint
        .images
        .iter()
        .map(|splice| (splice.key.get(), splice.cursor.as_slice()))
        .collect();
    let put_at = u64::try_from(data.len() + 2).expect("small");
    assert_eq!(splices, [(0, b"".as_slice()), (put_at, b"\x1b[2;1H")]);
}

#[test]
fn primary_images_are_spliced_into_the_primary_paint_while_the_alternate_is_active() {
    let image = kitty("a=T,i=1");
    let live = fed(&[&image, b"\x1b[?1049h\x1b[3;1H", SIXEL, b"\r\nvim"]);
    let paint = live.first_paint();
    let enter = paint
        .bytes
        .windows(8)
        .position(|window| window == b"\x1b[?1049h")
        .expect("the paint enters the alternate screen");
    let at: Vec<usize> = paint.images.iter().map(|splice| splice.at).collect();
    assert!(
        matches!(at.as_slice(), [primary, alternate] if *primary < enter && enter < *alternate)
    );
    let bytes_of = |key: u64| {
        if key == 0 {
            image.clone()
        } else {
            SIXEL.to_vec()
        }
    };
    let fresh = fed(&[&replayed(&live, bytes_of)]);
    assert_eq!(places(&fresh), places(&live));
    assert_eq!(fresh.screen().lines, live.screen().lines);
}

#[test]
fn an_image_off_the_first_column_is_replayed_at_its_column() {
    let image = kitty("a=T,i=1");
    let live = fed(&[b"ab\r\ncdef", &image]);
    let paint = live.first_paint();
    let cursors: Vec<&[u8]> = paint
        .images
        .iter()
        .map(|splice| splice.cursor.as_slice())
        .collect();
    assert_eq!(cursors, [b"\x1b[2;5H".as_slice()]);
    let fresh = fed(&[&replayed(&live, |_| image.clone())]);
    assert_eq!(places(&fresh), places(&live));
}

#[test]
fn an_erased_image_is_not_spliced() {
    let live = fed(&[&kitty("a=T,i=1"), b"x"]);
    assert_eq!(live.first_paint().images, Vec::new());
}
