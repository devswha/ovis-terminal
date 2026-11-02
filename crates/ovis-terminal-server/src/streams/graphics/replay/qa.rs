//! Retained images through a real attach socket after a reattach (plan todo 84).
//!
//! A viewer attaching late receives, after the grid paint, every graphics sequence that
//! is still on screen, byte-identical and in receipt order, and a `graphics_replayed`
//! record naming them. Images whose placement scrolled out, was erased or was deleted
//! by Kitty `a=d` are not re-sent and are named as released instead. A store at its cap
//! evicts the oldest image and reports typed counts in a `graphics_evicted` record.

use ovis_terminal_authz::Permission;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::ByteCount;
use ovis_terminal_server::streams::StreamLimits;
use ovis_terminal_vt::Vt;
use serde_json::{Value, json};

use super::TestResult;
use super::fixture::{Fixture, vt_config};
use super::socket::at;
use super::viewer::{Viewer, open_params};

const SESSION: u64 = 64 * 1024 * 1024;
const PNG: &[u8] = b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
const SIXEL: &[u8] = b"\x1bP0;1;0q\"1;1;6;6#0;2;100;0;0#0~~~~~~-\x1b\\";

fn kitty_series(id: u32) -> Vec<u8> {
    let (first, rest) = PNG.split_at(32);
    [
        format!("\x1b_Ga=T,f=100,i={id},m=1;").as_bytes(),
        first,
        b"\x1b\\\x1b_Gm=0;",
        rest,
        b"\x1b\\",
    ]
    .concat()
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn observer(fx: &Fixture, op: &str) -> TestResult<Viewer> {
    Viewer::open(
        fx.connect("phone").await?,
        &open_params(op, op, "observe"),
        true,
    )
    .await
}

/// A late observer that has read its whole paint: the program writes `marker` after it
/// attached, so everything before the marker is the paint.
async fn reattach(fx: &mut Fixture, op: &str, marker: &[u8]) -> TestResult<Viewer> {
    let mut late = observer(fx, op).await?;
    fx.publish(marker)?;
    late.until(|viewer| viewer.received.ends_with(marker))
        .await?;
    Ok(late)
}

fn replayed(viewer: &Viewer) -> TestResult<Value> {
    let record = viewer.first("graphics_replayed")?;
    Ok(at(&record, "/params/graphics_replayed").clone())
}

fn offset_of(fx: &Fixture, sequence: &[u8]) -> TestResult<String> {
    Ok(position(&fx.raw, sequence)
        .ok_or("sequence was not published")?
        .to_string())
}

fn released(notice: &Value) -> Vec<(String, String)> {
    at(notice, "/released")
        .as_array()
        .map(|list| {
            list.iter()
                .map(|item| {
                    (
                        at(item, "/output_offset").as_str().unwrap_or("").to_owned(),
                        at(item, "/reason").as_str().unwrap_or("").to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) async fn an_on_screen_image_is_replayed_byte_identical_after_a_reattach() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    // Given a program that drew a 2-chunk Kitty PNG and a sixel on two rows,
    let kitty = kitty_series(7);
    let raw = [b"top\r\n".as_slice(), &kitty, b"\r\n", SIXEL, b"\r\nshown"].concat();
    for piece in raw.chunks(7) {
        fx.publish(piece)?;
    }
    // When a viewer attaches afterwards,
    let late = reattach(&mut fx, "replay-1", b" after").await?;
    // Then the paint re-sends both sequences byte-identical, Kitty first, before live bytes.
    let kitty_at = position(&late.received, &kitty).ok_or("kitty image not replayed")?;
    let sixel_at = position(&late.received, SIXEL).ok_or("sixel not replayed")?;
    let live_at = position(&late.received, b" after").ok_or("live bytes")?;
    assert!(kitty_at < sixel_at && sixel_at < live_at);
    // And each is placed where its sequence began: row 2 and row 3, column 1.
    let before_kitty = late.received.get(..kitty_at).unwrap_or_default();
    let before_sixel = late.received.get(..sixel_at).unwrap_or_default();
    assert!(before_kitty.ends_with(b"\x1b[2;1H"));
    assert!(before_sixel.ends_with(b"\x1b[3;1H"));
    // And one record names both, in receipt order, with nothing released.
    let notice = replayed(&late)?;
    assert_eq!(
        at(&notice, "/replayed"),
        &json!([offset_of(&fx, &kitty)?, offset_of(&fx, SIXEL)?])
    );
    assert_eq!(released(&notice), Vec::<(String, String)>::new());
    // And the replay draws no cell: the grid equals a VT fed every raw byte.
    let mut continuous = Vt::new(vt_config()?);
    drop(continuous.feed(&fx.raw));
    assert_eq!(late.screen()?.screen().lines, continuous.screen().lines);
    assert_eq!(late.screen()?.screen().cursor, continuous.screen().cursor);
    fx.close().await
}

pub(crate) async fn a_scrolled_off_image_is_not_replayed() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    // Given an image in one read, then a sixel and more lines than the 8-row screen holds
    // in the next, so the sixel is already scrolled off when its bytes reach the store,
    let kitty = kitty_series(5);
    let lines = "line\r\n".repeat(12);
    fx.publish(&[kitty.as_slice(), b"\r\n"].concat())?;
    fx.publish(&[SIXEL, b"\r\n", lines.as_bytes()].concat())?;
    // When a viewer attaches,
    let late = reattach(&mut fx, "replay-2", b" after").await?;
    // Then neither is re-sent, and the record names both scrolled out.
    assert_eq!(position(&late.received, b"\x1b_G"), None);
    assert_eq!(position(&late.received, SIXEL), None);
    let notice = replayed(&late)?;
    assert_eq!(at(&notice, "/replayed"), &json!([]));
    let sixel = offset_of(&fx, SIXEL)?;
    assert_eq!(
        released(&notice),
        [
            ("0".to_owned(), "scrolled_out".to_owned()),
            (sixel, "scrolled_out".to_owned())
        ],
        "in receipt order, though the sixel was released first"
    );
    fx.close().await
}

pub(crate) async fn a_cleared_screen_replays_no_image() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    // Given two images on an otherwise blank screen, then ED 2,
    fx.publish(&[kitty_series(1).as_slice(), b"\r\n", SIXEL].concat())?;
    fx.publish(b"\x1b[2J")?;
    // When a viewer attaches,
    let late = reattach(&mut fx, "replay-3", b" after").await?;
    // Then neither is re-sent: alacritty's ED 2 moves the top row into scrollback and
    // resets the rest, so the first is named scrolled out and the second erased.
    assert_eq!(position(&late.received, b"\x1b_G"), None);
    assert_eq!(position(&late.received, SIXEL), None);
    let notice = replayed(&late)?;
    assert_eq!(at(&notice, "/replayed"), &json!([]));
    let reasons: Vec<String> = released(&notice).into_iter().map(|(_, why)| why).collect();
    assert_eq!(reasons, ["scrolled_out", "erased"]);
    fx.close().await
}

pub(crate) async fn a_kitty_delete_removes_only_the_named_image() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    // Given Kitty images 1 and 2 on two rows, then `a=d,d=I,i=1`,
    let (one, two) = (kitty_series(1), kitty_series(2));
    fx.publish(&[one.as_slice(), b"\r\n", &two, b"\r\n"].concat())?;
    fx.publish(b"\x1b_Ga=d,d=I,i=1\x1b\\")?;
    // When a viewer attaches,
    let late = reattach(&mut fx, "replay-4", b" after").await?;
    // Then image 2 is re-sent and image 1 is gone, named deleted.
    assert_eq!(position(&late.received, &one), None);
    assert!(position(&late.received, &two).is_some());
    let notice = replayed(&late)?;
    assert_eq!(at(&notice, "/replayed"), &json!([offset_of(&fx, &two)?]));
    assert_eq!(
        released(&notice),
        [(offset_of(&fx, &one)?, "deleted".to_owned())]
    );
    fx.close().await
}

pub(crate) async fn a_full_store_evicts_the_oldest_with_typed_counts() -> TestResult {
    // Given a store cap that holds two of these three images,
    let (one, two, three) = (kitty_series(1), kitty_series(2), kitty_series(3));
    let cap = one.len().saturating_mul(2).saturating_add(10);
    let mut limits = Limits::v1();
    limits.graphics_terminal_store_max_bytes = ByteCount::new(u64::try_from(cap)?);
    let mut fx = Fixture::new(StreamLimits::advertised(&limits), SESSION).await?;
    fx.issue("phone", &[Permission::Observe])?;
    let mut live = observer(&fx, "replay-5").await?;
    // When three images are drawn on three rows while a viewer watches,
    fx.publish(&[one.as_slice(), b"\r\n", &two, b"\r\n", &three, b"\r\ndone"].concat())?;
    live.until(|viewer| viewer.received.ends_with(b"done"))
        .await?;
    // Then the third evicts the first, reported once with typed counts,
    let evicted = live.kinds("graphics_evicted");
    assert_eq!(evicted.len(), 1, "{:?}", live.controls);
    let notice = at(
        evicted.first().copied().ok_or("evicted")?,
        "/params/graphics_evicted",
    );
    let size = one.len().to_string();
    assert_eq!(
        notice,
        &json!({
            "reason": "store_full",
            "evicted_sequences": "1",
            "evicted_bytes": size,
            "retained_bytes": one.len().saturating_mul(2).to_string(),
            "limit_bytes": cap.to_string(),
        })
    );
    // And a viewer attaching afterwards gets images 2 and 3 only, image 1 named store_full.
    let late = reattach(&mut fx, "replay-6", b" after").await?;
    assert_eq!(position(&late.received, &one), None);
    let replay = replayed(&late)?;
    assert_eq!(
        at(&replay, "/replayed"),
        &json!([offset_of(&fx, &two)?, offset_of(&fx, &three)?])
    );
    assert_eq!(
        released(&replay),
        [("0".to_owned(), "store_full".to_owned())]
    );
    fx.close().await
}
