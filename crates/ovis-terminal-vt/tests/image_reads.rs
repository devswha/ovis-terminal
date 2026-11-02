//! Screen reads name each image shown on the active screen by the cells it covers
//! (plan todo 85); text lines are what a text-only stream would show.

use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;
use ovis_terminal_vt::{Geometry, HistoryLimit, OutputOffset, ScreenImage, Vt, VtConfig};

type Cells = (u64, GraphicsProtocol, u16, u16, u16, u16);

#[cfg(test)]
fn engine(columns: u16, rows: u16) -> Vt {
    let geometry = Geometry::new(columns, rows).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(100)))
}

fn kitty(header: &str) -> Vec<u8> {
    format!("\x1b_G{header};AAAAAAAA\x1b\\").into_bytes()
}

fn cells(vt: &Vt) -> Vec<Cells> {
    vt.screen()
        .images
        .iter()
        .map(|image: &ScreenImage| {
            (
                image.key.get(),
                image.protocol,
                image.row,
                image.column,
                image.rows,
                image.columns,
            )
        })
        .collect()
}

#[test]
fn a_kitty_image_is_reported_at_its_cells_and_the_text_stays_what_text_alone_shows() {
    let mut vt = engine(20, 6);
    let mut text_only = engine(20, 6);
    for (part, image) in [
        (b"\r\nab".to_vec(), false),
        (kitty("a=T,f=100,i=7,c=4,r=2"), true),
        (b"\r\n\r\n\r\ndone".to_vec(), false),
    ] {
        drop(vt.feed(&part));
        if !image {
            drop(text_only.feed(&part));
        }
    }
    assert_eq!(cells(&vt), vec![(4, GraphicsProtocol::Kitty, 1, 2, 2, 4)]);
    assert_eq!(vt.screen().lines, text_only.screen().lines);
    assert_eq!(cells(&text_only), Vec::<Cells>::new());
}

#[test]
fn rows_scrolled_off_the_top_leave_the_rest_reported_until_none_is_visible() {
    let mut vt = engine(20, 4);
    drop(vt.feed(b"\r\n"));
    drop(vt.feed(&kitty("a=T,i=1,c=3,r=3")));
    drop(vt.feed(b"\r\n\r\n"));
    assert_eq!(cells(&vt), vec![(2, GraphicsProtocol::Kitty, 1, 0, 3, 3)]);
    drop(vt.feed(b"\r\n"));
    assert_eq!(cells(&vt), vec![(2, GraphicsProtocol::Kitty, 0, 0, 3, 3)]);
    drop(vt.feed(b"\r\n"));
    assert_eq!(cells(&vt), vec![(2, GraphicsProtocol::Kitty, 0, 0, 2, 3)]);
    drop(vt.feed(b"\r\n\r\n"));
    assert_eq!(cells(&vt), Vec::<Cells>::new());
    assert_eq!(vt.images().retained, Vec::new());
}

#[test]
fn printing_over_an_image_removes_its_placeholder() {
    let mut vt = engine(20, 4);
    drop(vt.feed(&kitty("a=T,i=1,c=2,r=1")));
    assert_eq!(cells(&vt).len(), 1);
    drop(vt.feed(b"\rXY"));
    assert_eq!(cells(&vt), Vec::<Cells>::new());
    assert_eq!(
        vt.screen().lines.first().map(|line| line.text.as_str()),
        Some("XY")
    );
}

#[test]
fn a_footprint_is_clipped_at_the_right_edge_and_the_bottom() {
    let mut vt = engine(20, 3);
    drop(vt.feed(b"\x1b[2;17H"));
    drop(vt.feed(&kitty("a=T,i=1,c=10,r=5")));
    assert_eq!(cells(&vt), vec![(7, GraphicsProtocol::Kitty, 1, 16, 2, 4)]);
}

#[test]
fn sixel_iterm2_and_kitty_puts_are_reported_but_a_bare_transmission_is_not() {
    let mut vt = engine(30, 6);
    let transmit = kitty("a=t,i=9");
    drop(vt.feed(&transmit));
    let put_at = vt.offset().get();
    drop(vt.feed(b"\x1b_Ga=p,i=9,c=2,r=1\x1b\\"));
    drop(vt.feed(b"\x1b[3;1H"));
    let sixel_at = vt.offset().get();
    drop(vt.feed(b"\x1bP0;1;0q\"1;1;6;6#0~~~~~~-\x1b\\"));
    drop(vt.feed(b"\x1b[5;4H"));
    let iterm_at = vt.offset().get();
    drop(vt.feed(b"\x1b]1337;File=inline=1;width=5;height=2:AAAA\x07"));
    assert_eq!(
        cells(&vt),
        vec![
            (put_at, GraphicsProtocol::Kitty, 0, 0, 1, 2),
            (sixel_at, GraphicsProtocol::Sixel, 2, 0, 1, 1),
            (iterm_at, GraphicsProtocol::Iterm2, 4, 3, 2, 5),
        ]
    );
    assert!(vt.images().retains(OutputOffset::new(0)));
}

#[test]
fn a_primary_screen_image_is_not_reported_on_the_alternate_screen_even_under_an_alternate_image() {
    let mut vt = engine(20, 4);
    drop(vt.feed(b"\r\n"));
    drop(vt.feed(&kitty("a=T,i=1,c=2,r=1")));
    let primary = cells(&vt);
    assert_eq!(primary, vec![(2, GraphicsProtocol::Kitty, 1, 0, 1, 2)]);
    drop(vt.feed(b"\x1b[?1049h"));
    assert_eq!(cells(&vt), Vec::<Cells>::new());
    let on_alternate = vt.offset().get();
    drop(vt.feed(&kitty("a=T,i=2,c=2,r=1")));
    assert_eq!(
        cells(&vt),
        vec![(on_alternate, GraphicsProtocol::Kitty, 1, 0, 1, 2)]
    );
    drop(vt.feed(b"\x1b[?1049l"));
    assert_eq!(cells(&vt), primary);
}
