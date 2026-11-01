//! Graphics sequences through a real attach socket (plan todo 83).
//!
//! A credited observer receives Kitty, iTerm2 and sixel sequences byte-identical, an
//! admitted sequence larger than the 4 MiB stream backlog arrives whole without a gap,
//! and an over-cap Kitty series is replaced by exactly one `graphics_dropped` record
//! while the grid stays what the VT drew from every raw byte.

use ovis_terminal_authz::Permission;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::StreamLimits;
use ovis_terminal_vt::Vt;
use sha2::{Digest, Sha256};

use super::TestResult;
use super::fixture::{Fixture, vt_config};
use super::socket::at;
use super::viewer::{Viewer, open_params};

const CAP: usize = 8 * 1024 * 1024;
const SESSION: u64 = 64 * 1024 * 1024;
const PNG: &[u8] = b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

fn kitty_series() -> Vec<u8> {
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

const SIXEL: &[u8] = b"\x1bP0;1;0q\"1;1;6;6#0;2;100;0;0#0~~~~~~-\x1b\\";

fn iterm2() -> Vec<u8> {
    [b"\x1b]1337;File=inline=1;size=70:".as_slice(), PNG, b"\x07"].concat()
}

/// One Kitty transmission of exactly `len` raw bytes.
fn kitty_of(len: usize) -> Vec<u8> {
    let head = b"\x1b_Ga=T,f=100,i=9;";
    let payload = len.saturating_sub(head.len()).saturating_sub(2);
    [head.as_slice(), &vec![b'A'; payload], b"\x1b\\"].concat()
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

async fn observer(fx: &Fixture, op: &str) -> TestResult<Viewer> {
    Viewer::open(
        fx.connect("phone").await?,
        &open_params(op, op, "observe"),
        true,
    )
    .await
}

/// Publishes `bytes` cut into reads of `read` bytes.
fn publish_in(fx: &mut Fixture, bytes: &[u8], read: usize) -> TestResult {
    for piece in bytes.chunks(read) {
        fx.publish(piece)?;
    }
    Ok(())
}

pub(crate) async fn three_protocols_reach_a_live_viewer_byte_identical() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    let mut viewer = observer(&fx, "graphics-1").await?;
    // Given a program writing a 3-chunk Kitty PNG, a sixel and an iTerm2 image between text,
    let live = [
        b"before ".as_slice(),
        &kitty_series(),
        SIXEL,
        &iterm2(),
        b" after",
    ]
    .concat();
    // When its output arrives in 5-byte reads that cut every sequence,
    publish_in(&mut fx, &live, 5)?;
    viewer
        .until(|viewer| viewer.received.ends_with(b" after"))
        .await?;
    // Then the viewer's live bytes are exactly the program's bytes, in order.
    let tail = viewer
        .received
        .get(viewer.received.len().saturating_sub(live.len())..)
        .unwrap_or_default();
    assert_eq!(sha(tail), sha(&live));
    for sequence in [kitty_series(), SIXEL.to_vec(), iterm2()] {
        assert!(contains(&viewer.received, &sequence));
    }
    assert_eq!(viewer.kinds("gap"), Vec::<&serde_json::Value>::new());
    assert_eq!(
        viewer.kinds("graphics_dropped"),
        Vec::<&serde_json::Value>::new()
    );
    fx.close().await
}

pub(crate) async fn a_sequence_at_the_cap_arrives_whole_past_the_stream_backlog() -> TestResult {
    let limits = StreamLimits::advertised(&Limits::v1());
    let mut fx = Fixture::new(limits, SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    let mut viewer = observer(&fx, "graphics-2").await?;
    // Given one Kitty transmission of exactly the 8 MiB cap, twice the 4 MiB backlog,
    let image = kitty_of(CAP);
    let live = [image.as_slice(), b"done"].concat();
    // When it is published in 64 KiB reads faster than the viewer reads,
    publish_in(&mut fx, &live, 64 * 1024)?;
    viewer
        .until(|viewer| viewer.received.ends_with(b"done"))
        .await?;
    // Then it arrives whole and in order, with no gap and no notice.
    assert_eq!(viewer.kinds("gap"), Vec::<&serde_json::Value>::new());
    assert_eq!(
        viewer.kinds("graphics_dropped"),
        Vec::<&serde_json::Value>::new()
    );
    let tail = viewer
        .received
        .get(viewer.received.len().saturating_sub(live.len())..)
        .unwrap_or_default();
    assert_eq!(sha(tail), sha(&live));
    fx.close().await
}

pub(crate) async fn an_over_cap_series_is_replaced_by_one_typed_notice() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    let mut viewer = observer(&fx, "graphics-3").await?;
    // Given a Kitty series whose chunks together pass the cap by one byte,
    let half = CAP / 2;
    let first = [
        b"\x1b_Ga=T,f=100,i=3,m=1;".as_slice(),
        &vec![b'A'; half],
        b"\x1b\\",
    ]
    .concat();
    let rest = CAP.saturating_add(1).saturating_sub(first.len());
    let second = [
        b"\x1b_Gm=0;".as_slice(),
        &vec![b'B'; rest.saturating_sub(9)],
        b"\x1b\\",
    ]
    .concat();
    let series = [first, second].concat();
    assert_eq!(series.len(), CAP.saturating_add(1));
    let raw = [b"before".as_slice(), &series, b"after\r\n"].concat();
    publish_in(&mut fx, &raw, 64 * 1024)?;
    viewer
        .until(|viewer| viewer.received.ends_with(b"after\r\n"))
        .await?;
    // Then exactly one notice stands in its place and no payload byte was sent.
    let notices = viewer.kinds("graphics_dropped");
    assert_eq!(notices.len(), 1, "{:?}", viewer.controls);
    let params = notices.first().copied().ok_or("notice")?;
    let notice = &params["params"];
    assert_eq!(at(notice, "/graphics_dropped/reason"), "too_large");
    assert_eq!(at(notice, "/graphics_dropped/protocol"), "kitty");
    assert_eq!(at(notice, "/graphics_dropped/limit_bytes"), "8388608");
    assert_eq!(at(notice, "/graphics_dropped/sequence_bytes"), "8388609");
    assert_eq!(at(notice, "/graphics_dropped/output_offset"), "6");
    assert!(viewer.received.ends_with(b"beforeafter\r\n"));
    assert!(!contains(&viewer.received, b"\x1b_G"));
    assert!(!contains(&viewer.received, b"AAAA") && !contains(&viewer.received, b"BBBB"));
    // And a viewer attaching afterwards paints the grid a VT drew from every raw byte.
    let mut late = observer(&fx, "graphics-4").await?;
    late.until(|viewer| contains(&viewer.received, b"beforeafter"))
        .await?;
    let mut continuous = Vt::new(vt_config()?);
    drop(continuous.feed(&fx.raw));
    let mut text_only = Vt::new(vt_config()?);
    drop(text_only.feed(b"beforeafter\r\n"));
    let painted = late.screen()?;
    assert_eq!(painted.screen().lines, continuous.screen().lines);
    assert_eq!(painted.screen().lines, text_only.screen().lines);
    assert_eq!(painted.screen().cursor, text_only.screen().cursor);
    fx.close().await
}
