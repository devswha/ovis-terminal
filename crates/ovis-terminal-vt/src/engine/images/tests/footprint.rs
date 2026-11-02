//! A placement is its whole declared footprint, not its anchor cell.

use super::{engine, keys, kitty, released};
use crate::engine::Vt;
use crate::engine::images::ImageRelease;

fn fed(chunks: &[&[u8]]) -> Vt {
    let mut vt = engine();
    for chunk in chunks {
        drop(vt.feed(chunk));
    }
    vt
}

#[test]
fn a_write_inside_the_footprint_but_off_the_anchor_expires_the_image() {
    let image = kitty("a=T,i=1,r=2,c=5");
    let inside = fed(&[&image, b"\x1b[2;2Hx"]);
    assert_eq!(keys(&inside), Vec::<u64>::new());
    assert_eq!(released(&inside, 0), Some(ImageRelease::Erased));
    assert_eq!(inside.first_paint().images, Vec::new());
    let erased = fed(&[&image, b"\x1b[2;3H\x1b[K"]);
    assert_eq!(released(&erased, 0), Some(ImageRelease::Erased));
    let outside = fed(&[&image, b"\x1b[2;6Hx\x1b[3;1Hx"]);
    assert_eq!(keys(&outside), [0], "cells beside and below the footprint");
}

#[test]
fn an_image_survives_while_any_of_its_rows_is_on_screen_and_replays_there() {
    let mut vt = fed(&[&kitty("a=T,i=1,r=2,c=5"), b"\r\n\r\n\r\n\r\n"]);
    assert_eq!(keys(&vt), [0], "its second row is still the top row");
    let place = vt.images().retained.first().and_then(|image| image.place);
    assert_eq!(place.map(|place| (place.row, place.rows)), Some((0, 2)));
    let cursor: Vec<Vec<u8>> = vt
        .first_paint()
        .images
        .iter()
        .map(|splice| splice.cursor.clone())
        .collect();
    assert_eq!(
        cursor,
        [b"\x1b[1;1H".to_vec()],
        "replayed at the clamped row"
    );
    drop(vt.feed(b"\r\n"));
    assert_eq!(keys(&vt), Vec::<u64>::new());
    assert_eq!(released(&vt, 0), Some(ImageRelease::ScrolledOut));
}

#[test]
fn an_inserted_line_inside_the_image_splits_and_expires_it() {
    let vt = fed(&[&kitty("a=T,i=1,r=2,c=5"), b"\x1b[2;1H\x1b[Lhello"]);
    assert_eq!(keys(&vt), Vec::<u64>::new());
    assert_eq!(released(&vt, 0), Some(ImageRelease::Erased));
    assert_eq!(
        vt.first_paint().images,
        Vec::new(),
        "never painted over hello"
    );
}

#[test]
fn a_region_scroll_down_inside_the_image_splits_and_expires_it() {
    let vt = fed(&[&kitty("a=T,i=1,r=2,c=5"), b"\x1b[2;4r\x1b[1T\x1b[2;1Hhello"]);
    assert_eq!(keys(&vt), Vec::<u64>::new());
    assert_eq!(released(&vt, 0), Some(ImageRelease::Erased));
    assert_eq!(vt.first_paint().images, Vec::new());
}

#[test]
fn a_whole_image_moved_by_a_region_scroll_stays() {
    let vt = fed(&[
        b"\x1b[2;1H",
        &kitty("a=T,i=1,r=2,c=5"),
        b"\x1b[1;4r\x1b[1;1H\x1b[L",
    ]);
    let place = vt.images().retained.first().and_then(|image| image.place);
    assert_eq!(
        place.map(|place| place.row),
        Some(2),
        "both rows moved down together"
    );
}

#[test]
fn a_footprint_is_clipped_to_the_grid() {
    let image = kitty("a=T,i=1,r=9,c=9");
    let vt = fed(&[b"\x1b[4;18H", &image]);
    assert_eq!(
        keys(&vt),
        [7],
        "rows and columns past the grid are dropped, not refused"
    );
    let written = fed(&[b"\x1b[4;18H", &image, b"\x1b[4;20Hx"]);
    assert_eq!(released(&written, 7), Some(ImageRelease::Erased));
}
