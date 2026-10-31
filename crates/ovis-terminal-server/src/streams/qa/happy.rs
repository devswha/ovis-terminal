//! Late first paint parity, held with an actionable retry, isolated credit, final bytes.

use ovis_terminal_authz::Permission;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_server::streams::StreamLimits;
use ovis_terminal_vt::Vt;
use serde_json::{Value, json};

use super::TestResult;
use super::fixture::{Fixture, vt_config};
use super::socket::{at, error_kind};
use super::viewer::{Viewer, open_params};

const OBSERVER: &[Permission] = &[Permission::Observe];
const INTERACTOR: &[Permission] = &[Permission::Observe, Permission::Interact];
const WINDOW: usize = 256 * 1024;
const QUERY: &[u8] = b"\x1b[6n";
const REFUSAL_TEXT: &[u8] = b"herdr: server shut down: attach refused";

fn text(value: &Value, pointer: &str) -> String {
    at(value, pointer).as_str().unwrap_or_default().to_owned()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn flood() -> Vec<u8> {
    (0..12_000)
        .flat_map(|line: u32| format!("line {line:05} of the build log\r\n").into_bytes())
        .collect()
}

async fn open(
    fx: &Fixture,
    (label, op): (&str, &str),
    (client, role): (&str, &str),
) -> TestResult<Viewer> {
    Viewer::open(
        fx.connect(label).await?,
        &open_params(op, client, role),
        true,
    )
    .await
}

/// The operator attaches with interact after content was painted once.
async fn operator_is_ready(fx: &mut Fixture) -> TestResult<Viewer> {
    // Given content painted once before anyone attached, with a title and paste mode.
    fx.publish(b"\x1b]0;build\x07\x1b[?2004h\x1b[1;32mready\x1b[0m\r\n$ make\r\n")?;
    let painted_once = fx.raw.len().to_string();
    // When the operator attaches with interact.
    let mut operator = open(fx, ("owner", "op-1"), ("tui", "interact")).await?;
    let opened = operator.result.clone();
    assert_eq!(text(&opened, "/result/outcome"), "ready");
    assert_eq!(text(&opened, "/result/lease/role"), "interact");
    assert_eq!(text(&opened, "/result/terminal/terminal_id"), "terminal:7");
    assert_eq!(text(&opened, "/result/terminal/terminal_incarnation"), "1");
    assert_eq!(text(&opened, "/result/terminal/process_incarnation"), "1");
    assert_eq!(text(&opened, "/result/credit_initial"), WINDOW.to_string());
    assert_eq!(
        text(&opened, "/result/first_paint/output_offset"),
        painted_once
    );
    assert_eq!(text(&opened, "/result/first_paint/stream_offset"), "0");
    operator
        .until(|viewer| !viewer.kinds("input_ready").is_empty())
        .await?;
    // Then the checkpoint record precedes every byte and repeats the reply's paint.
    let first = operator.controls.first().ok_or("no control record")?;
    assert_eq!(at(first, "/params/kind"), "checkpoint");
    assert_eq!(
        at(first, "/params/checkpoint"),
        at(&opened, "/result/first_paint")
    );
    Ok(operator)
}

/// A late observer is ready; a second interact client is held with an actionable boundary.
async fn late_and_held(fx: &mut Fixture, operator: &Viewer) -> TestResult<(Viewer, Viewer)> {
    // When a cursor query is split across reads, the core answers it once.
    assert_eq!(fx.publish(b"compile 1\x1b[6")?.replies.len(), 0);
    assert_eq!(fx.publish(b"n compiled\r\n")?.replies.len(), 1);
    // And a phone attaches late, after content that was painted only once.
    let phone = open(fx, ("phone", "op-2"), ("phone", "observe")).await?;
    assert_eq!(text(&phone.result, "/result/outcome"), "ready");
    let at_attach = text(&phone.result, "/result/first_paint/output_offset");
    assert_eq!(at_attach, fx.raw.len().to_string());
    assert_ne!(phone.stream, operator.stream, "stream identity is fresh");
    // When a second interact client attaches, it is held with the owner and the boundary.
    let held = open(fx, ("bridge", "op-3"), ("bridge", "interact")).await?;
    let lease = at(&operator.result, "/result/lease");
    assert_eq!(text(&held.result, "/result/outcome"), "held");
    assert_eq!(text(&held.result, "/result/holder/holder"), "tui");
    assert_eq!(at(&held.result, "/result/holder"), lease);
    assert_eq!(text(&held.result, "/result/retry/on"), "lease_released");
    assert_eq!(text(&held.result, "/result/retry/event"), "lease.revoked");
    assert_eq!(at(&held.result, "/result/retry/lease"), lease);
    assert_eq!(
        text(&held.result, "/result/lease/role"),
        "observe",
        "held still paints"
    );
    Ok((phone, held))
}

/// A viewer that grants no credit receives exactly its window while output floods.
async fn slow_stops_at_its_window(fx: &mut Fixture) -> TestResult<Viewer> {
    let client = fx.connect("slow").await?;
    let mut slow = Viewer::open(client, &open_params("op-4", "slow", "observe"), false).await?;
    for chunk in flood().chunks(32 * 1024) {
        fx.publish(chunk)?;
    }
    fx.publish(&[b"\x1b[2;1Hpatched\r\n".as_slice(), REFUSAL_TEXT, b"\r\n"].concat())?;
    // Then the slow viewer is held at exactly its window: one more byte was never sent.
    slow.until(|viewer| viewer.received.len() >= WINDOW).await?;
    let beyond =
        json!({"stream_id": slow.stream, "acknowledged": WINDOW.saturating_add(1).to_string()});
    assert_eq!(
        error_kind(&slow.call("attach.credit", &beyond).await?),
        "invalid_credit"
    );
    assert_eq!(slow.received.len(), WINDOW, "exactly the window was sent");
    Ok(slow)
}

/// The holder releases; CLOSE follows the committed revocation and the retry is ready.
async fn release_then_retry(fx: &Fixture, mut operator: Viewer) -> TestResult {
    let closed = operator
        .call("attach.close", &json!({"stream_id": operator.stream}))
        .await?;
    assert!(closed.get("result").is_some(), "{closed}");
    operator
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    assert_eq!(
        text(&operator.first("close")?, "/params/close/reason"),
        "client_closed"
    );
    fx.journal.revoked("tui").await?;
    let retried = open(fx, ("bridge", "op-5"), ("bridge", "interact")).await?;
    assert_eq!(text(&retried.result, "/result/outcome"), "ready");
    Ok(())
}

async fn closes_after_final_bytes(viewer: &mut Viewer, raw_end: &str) -> TestResult {
    viewer
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let close = viewer.first("close")?;
    assert_eq!(text(&close, "/params/close/reason"), "exited");
    assert_eq!(at(&close, "/params/close/exit_code"), 0);
    assert_eq!(text(&close, "/params/close/final_output_offset"), raw_end);
    let delivered = viewer.received.len().to_string();
    assert_eq!(text(&close, "/params/close/final_offset"), delivered);
    // Acknowledgements racing the close are refused as stale, never applied elsewhere.
    let stale = viewer
        .failures
        .iter()
        .all(|reply| error_kind(reply) == "invalid_credit");
    assert!(stale, "{:?}", viewer.failures);
    Ok(())
}

/// Paint plus live bytes reproduce a VT that saw every raw byte, minus answered queries.
fn matches_continuous_vt(viewer: &Viewer, raw: &[u8]) -> TestResult {
    let mut continuous = Vt::new(vt_config()?);
    drop(continuous.feed(raw));
    let screen = viewer.screen()?;
    assert_eq!(screen.screen().lines, continuous.screen().lines);
    assert_eq!(screen.screen().cursor, continuous.screen().cursor);
    assert_eq!(screen.screen().title, continuous.screen().title);
    assert_eq!(screen.modes(), continuous.modes());
    assert!(
        !contains(&viewer.received, QUERY),
        "an answered query never reaches a viewer"
    );
    assert!(contains(raw, QUERY), "raw output keeps the query");
    assert!(
        contains(&viewer.received, REFUSAL_TEXT),
        "refusal-like text stays data"
    );
    Ok(())
}

pub(crate) async fn happy() -> TestResult {
    let mut fx = Fixture::new(StreamLimits::advertised(&Limits::v1()), 64 * 1024 * 1024).await?;
    fx.issue("phone", OBSERVER)?;
    fx.issue("bridge", INTERACTOR)?;
    fx.issue("slow", OBSERVER)?;
    let operator = operator_is_ready(&mut fx).await?;
    let (mut phone, mut held) = late_and_held(&mut fx, &operator).await?;
    let mut slow = slow_stops_at_its_window(&mut fx).await?;
    release_then_retry(&fx, operator).await?;
    // When the program exits, credited viewers get every final byte, then CLOSE `exited`,
    // while the slow viewer still has not acknowledged anything.
    fx.exit(0)?;
    let raw_end = fx.raw.len().to_string();
    for viewer in [&mut phone, &mut held] {
        closes_after_final_bytes(viewer, &raw_end).await?;
        matches_continuous_vt(viewer, &fx.raw)?;
    }
    assert_eq!(slow.received.len(), WINDOW);
    // When the slow viewer finally acknowledges, it catches up and closes the same way.
    slow.credit = true;
    slow.acknowledge().await?;
    closes_after_final_bytes(&mut slow, &raw_end).await?;
    matches_continuous_vt(&slow, &fx.raw)?;
    fx.close().await
}
