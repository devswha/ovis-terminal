//! Which images the engine retains, where it places them and why it releases them.

mod footprint;
mod paint;

use alacritty_terminal::term::cell::Flags;
use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;

use super::anchor::MARK;
use super::{IMAGE_LIMIT, ImageRelease};
use crate::engine::Vt;
use crate::engine::modes::ScreenBuffer;
use crate::engine::units::{Geometry, HistoryLimit, OutputOffset, VtConfig};

pub(super) const SIXEL: &[u8] = b"\x1bP0;1;0q\"1;1;6;6#0~~~~~~-\x1b\\";

pub(super) fn engine() -> Vt {
    let geometry = Geometry::new(20, 4).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(50)))
}

pub(super) fn kitty(keys: &str) -> Vec<u8> {
    format!("\x1b_G{keys};AAAA\x1b\\").into_bytes()
}

pub(super) fn keys(vt: &Vt) -> Vec<u64> {
    vt.images()
        .retained
        .iter()
        .map(|image| image.key.get())
        .collect()
}

pub(super) fn released(vt: &Vt, key: u64) -> Option<ImageRelease> {
    vt.images().release_of(OutputOffset::new(key))
}

fn fed(chunks: &[&[u8]]) -> Vt {
    let mut vt = engine();
    for chunk in chunks {
        drop(vt.feed(chunk));
    }
    vt
}

#[test]
fn the_mark_is_a_flag_bit_alacritty_never_defines() {
    assert!(!Flags::all().intersects(MARK));
}

#[test]
fn an_image_is_placed_on_the_cursor_cell_it_arrived_at_with_its_header_extent() {
    let vt = fed(&[b"ab\r\ncd", &kitty("a=T,f=100,i=3,r=2,c=5")]);
    let images = vt.images();
    let image = images.retained.first().expect("one image");
    assert_eq!(images.retained.len(), 1);
    assert_eq!(
        (image.key.get(), image.protocol),
        (6, GraphicsProtocol::Kitty)
    );
    let place = image.place.expect("placed");
    assert_eq!(
        (
            place.buffer,
            place.row,
            place.column,
            place.rows,
            place.columns
        ),
        (ScreenBuffer::Primary, 1, 2, 2, 5)
    );
}

#[test]
fn chunking_never_changes_which_images_are_retained_or_where() {
    let stream = [
        b"top\r\n".as_slice(),
        &kitty("a=T,i=1,m=1"),
        &kitty("m=0"),
        b"\r\n",
        SIXEL,
        b"\r\n\x1b]1337;File=inline=1;width=3;height=2:QUJD\x07\r\n x",
    ]
    .concat();
    let whole = fed(&[&stream]);
    let mut bytewise = engine();
    for byte in &stream {
        drop(bytewise.feed(std::slice::from_ref(byte)));
    }
    assert_eq!(keys(&whole).len(), 3);
    assert_eq!(whole.images(), bytewise.images());
}

#[test]
fn a_kitty_series_is_one_image_keyed_by_its_first_chunk_even_when_interrupted() {
    let vt = fed(&[&kitty("a=T,i=1,m=1"), b"\r\n", &kitty("m=1"), &kitty("m=0")]);
    assert_eq!(keys(&vt), [0]);
    let after = fed(&[
        &kitty("a=T,i=1,m=1"),
        &kitty("m=0"),
        b"\r\n",
        &kitty("a=T,i=2"),
    ]);
    assert_eq!(
        keys(&after).len(),
        2,
        "a closed series does not swallow the next image"
    );
}

#[test]
fn the_anchor_follows_scrolling_and_leaves_with_the_visible_screen() {
    let mut vt = fed(&[b"a\r\nb\r\n", &kitty("a=T,i=1"), b"\r\n\r\n"]);
    let row = |vt: &Vt| {
        vt.images()
            .retained
            .first()
            .and_then(|image| image.place)
            .map(|p| p.row)
    };
    assert_eq!(
        row(&vt),
        Some(1),
        "one line scrolled the image from row 2 to row 1"
    );
    drop(vt.feed(b"\r\n"));
    assert_eq!(row(&vt), Some(0));
    drop(vt.feed(b"\r\n"));
    assert_eq!(keys(&vt), Vec::<u64>::new());
    assert_eq!(released(&vt, 6), Some(ImageRelease::ScrolledOut));
}

#[test]
fn erasing_or_printing_over_the_anchor_cell_releases_it_and_neighbours_do_not() {
    // alacritty's ED 2 moves the viewport into scrollback (`Grid::clear_viewport`).
    for (after, reason) in [
        (b"\x1b[2J".as_slice(), Some(ImageRelease::ScrolledOut)),
        (b"\x1b[2K", Some(ImageRelease::Erased)),
        (b"\x1b[X", Some(ImageRelease::Erased)),
        (b"x", Some(ImageRelease::Erased)),
        (b"\x1b[1;2Hx\x1b[2;1Hx", None),
    ] {
        let vt = fed(&[&kitty("a=T,i=1"), after]);
        assert_eq!(released(&vt, 0), reason, "{after:?}");
        assert_eq!(keys(&vt).len(), usize::from(reason.is_none()), "{after:?}");
    }
}

#[test]
fn a_region_scroll_moves_a_surviving_anchor_and_releases_a_recycled_one() {
    let moved = fed(&[b"\x1b[2;1H", &kitty("a=T,i=1"), b"\x1b[1;1H\x1b[M"]);
    let place = moved
        .images()
        .retained
        .first()
        .and_then(|image| image.place);
    assert_eq!(place.map(|place| place.row), Some(0));
    // A region starting below the top row recycles deleted rows instead of keeping them
    // as scrollback.
    let region = b"\x1b[2;4r\x1b[2;1H";
    let recycled = fed(&[region, &kitty("a=T,i=1"), b"\x1b[M"]);
    let key = u64::try_from(region.len()).expect("small");
    assert_eq!(released(&recycled, key), Some(ImageRelease::Erased));
}

#[test]
fn kitty_deletes_name_ids_numbers_and_placements_or_data_by_case() {
    let base = [
        kitty("a=t,i=1"),
        kitty("a=T,i=2"),
        b"\r\n".to_vec(),
        kitty("a=p,i=1"),
    ]
    .concat();
    let shown = u64::try_from(kitty("a=t,i=1").len()).expect("small");
    let put = u64::try_from(base.len() - kitty("a=p,i=1").len()).expect("small");
    let by_id = fed(&[&base, &kitty("a=d,d=i,i=2")]);
    assert_eq!(keys(&by_id), [0, put]);
    assert_eq!(released(&by_id, shown), Some(ImageRelease::Deleted));
    let placements = fed(&[&base, &kitty("a=d")]);
    assert_eq!(
        keys(&placements),
        [0],
        "lower case keeps the transmitted data"
    );
    let everything = fed(&[&base, &kitty("a=d,d=A")]);
    assert_eq!(keys(&everything), Vec::<u64>::new());
}

#[test]
fn a_transmission_reusing_an_id_replaces_the_old_image() {
    let first = kitty("a=T,i=1");
    let vt = fed(&[&first, b"\r\n", &first]);
    let second = u64::try_from(first.len() + 2).expect("small");
    assert_eq!(keys(&vt), [second]);
    assert_eq!(released(&vt, 0), Some(ImageRelease::Replaced));
}

#[test]
fn ris_releases_every_image() {
    let vt = fed(&[&kitty("a=T,i=1"), b"\r\n", SIXEL, b"\x1bc"]);
    assert_eq!(keys(&vt), Vec::<u64>::new());
    assert_eq!(released(&vt, 0), Some(ImageRelease::Reset));
}

#[test]
fn only_displayed_graphics_are_retained_never_their_lookalikes() {
    let shown = fed(&[b"\x1b]1337;File=name=eA==;inline=1;width=4;height=2:QUJD\x1b\\"]);
    let place = shown
        .images()
        .retained
        .first()
        .and_then(|image| image.place);
    assert_eq!(place.map(|place| (place.rows, place.columns)), Some((2, 4)));
    for lookalike in [
        b"\x1b]1337;File=inline=0:QUJD\x07".as_slice(),
        b"\x1b]1337;SetUserVar=a=Yg==\x07",
        b"\x1bP$q m\x1b\\",
        b"\x1b_Ga=q,i=4;AAAA\x1b\\",
    ] {
        assert_eq!(keys(&fed(&[lookalike])), Vec::<u64>::new(), "{lookalike:?}");
    }
    let long = [b"\x1bP".as_slice(), &[b'1'; 63], b"q#0~-\x1b\\"].concat();
    assert_eq!(
        keys(&fed(&[&long])),
        [0],
        "63 parameter bytes are still sixel"
    );
    let longer = [b"\x1bP".as_slice(), &[b'1'; 64], b"q#0~-\x1b\\"].concat();
    assert_eq!(keys(&fed(&[&longer])), Vec::<u64>::new());
}

#[test]
fn leaving_the_alternate_screen_releases_its_images_and_keeps_the_primary_ones() {
    let mut vt = fed(&[&kitty("a=T,i=1"), b"\x1b[?1049h\x1b[3;1H", SIXEL]);
    assert_eq!(keys(&vt).len(), 2);
    drop(vt.feed(b"\x1b[?1049l"));
    assert_eq!(keys(&vt), [0]);
}

#[test]
fn the_engine_retains_at_most_its_image_limit() {
    let stream: Vec<u8> = (0..=IMAGE_LIMIT)
        .flat_map(|index| kitty(&format!("a=t,i={index}")))
        .collect();
    let vt = fed(&[&stream]);
    assert_eq!(keys(&vt).len(), IMAGE_LIMIT);
    assert_eq!(released(&vt, 0), Some(ImageRelease::Limit));
}
