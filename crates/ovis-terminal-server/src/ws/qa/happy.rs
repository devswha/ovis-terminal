//! WebSocket and local-socket viewers of one terminal see the same stream.
//!
//! Two authenticated WebSocket viewers and a local-socket viewer share the same ordered
//! output, first paint, replay and close; hello reports the same identity and limits on
//! both transports; a reconnect attaches afresh.

use ovis_terminal_authz::Permission;
use ovis_terminal_vt::Vt;
use serde_json::{Value, json};

use super::TestResult;
use super::client::{PING, Received};
use super::fixture::{Fixture, Transport, vt_config};
use super::socket::{Client, at, error_kind, request};
use super::viewer::{Viewer, open_params};

const OBSERVER: &[Permission] = &[Permission::Observe];
const FOREVER: u64 = u64::MAX;
const LAST_CHUNK: &[u8] = b"\x1b[2;1Hpatched\r\n";

/// The hello result without the two fields that name the transport and the instant.
fn transport_neutral(hello: &Value) -> Value {
    let mut result = at(hello, "/result").clone();
    if let Some(fields) = result.as_object_mut() {
        fields.remove("endpoint_kind");
        fields.remove("server_time_ms");
    }
    result
}

/// Identity, limits (heartbeat and replay bounds included) and methods match; a ping is
/// answered on the transport and never becomes a protocol record.
async fn hello_and_keepalive_match(fx: &Fixture) -> TestResult {
    let owner = fx.bearer("owner")?;
    let local = fx.open(Transport::Unix).await?.hello(&owner).await?;
    let mut ws = fx.open(Transport::Ws).await?;
    let remote = ws.hello(&owner).await?;
    assert_eq!(at(&local, "/result/endpoint_kind"), "unix");
    assert_eq!(at(&remote, "/result/endpoint_kind"), "websocket");
    assert_eq!(transport_neutral(&local), transport_neutral(&remote));
    for limit in [
        "heartbeat_ms",
        "event_replay_events",
        "connection_unwritten_bytes",
    ] {
        let advertised = at(&remote, &format!("/result/limits/{limit}"));
        assert!(
            advertised.is_string(),
            "{limit} is a decimal string: {advertised}"
        );
    }
    let Client::Ws(socket) = &mut ws else {
        return Err("not a WebSocket client".into());
    };
    socket.send(PING, b"beat-1").await?;
    assert_eq!(
        socket.next().await?,
        Some(Received::Pong(b"beat-1".to_vec()))
    );
    let probe = json!({"stream_id": "stream:none", "acknowledged": "0"});
    ws.send(&request("after-ping", "attach.credit", &probe))
        .await?;
    let reply = ws.expect_next().await?;
    assert_eq!(reply.get("id"), Some(&json!("after-ping")), "{reply}");
    assert_eq!(error_kind(&reply), "stream_not_found");
    Ok(())
}

async fn attach(
    fx: &Fixture,
    (label, transport): (&str, Transport),
    op: &str,
) -> TestResult<Viewer> {
    let role = if label == "owner" {
        "interact"
    } else {
        "observe"
    };
    let client = fx.connect(label, transport).await?;
    let viewer = Viewer::open(client, &open_params(op, label, role), true).await?;
    assert_eq!(
        at(&viewer.result, "/result/outcome"),
        "ready",
        "{}",
        viewer.result
    );
    Ok(viewer)
}

async fn repaint(viewer: &mut Viewer) -> TestResult<Value> {
    let reply = viewer
        .call("attach.repaint", &json!({"stream_id": viewer.stream}))
        .await?;
    Ok(at(&reply, "/result/checkpoint").clone())
}

/// Paint and live bytes reproduce the VT that saw every raw byte.
fn matches_continuous_vt(viewer: &Viewer, raw: &[u8]) -> TestResult {
    let mut continuous = Vt::new(vt_config()?);
    drop(continuous.feed(raw));
    let screen = viewer.screen()?;
    assert_eq!(screen.screen().lines, continuous.screen().lines);
    assert_eq!(screen.screen().cursor, continuous.screen().cursor);
    assert_eq!(screen.screen().title, continuous.screen().title);
    assert_eq!(screen.modes(), continuous.modes());
    Ok(())
}

async fn closes_exited(viewer: &mut Viewer, raw_end: &str) -> TestResult {
    viewer
        .until(|viewer| !viewer.kinds("close").is_empty())
        .await?;
    let close = viewer.first("close")?;
    assert_eq!(at(&close, "/params/close/reason"), "exited");
    assert_eq!(at(&close, "/params/close/final_output_offset"), raw_end);
    let delivered = viewer.received.len().to_string();
    assert_eq!(at(&close, "/params/close/final_offset"), delivered.as_str());
    Ok(())
}

pub(crate) async fn happy() -> TestResult {
    let mut fx = Fixture::new().await?;
    fx.issue("phone", (OBSERVER, FOREVER))?;
    fx.issue("bridge", (OBSERVER, FOREVER))?;
    hello_and_keepalive_match(&fx).await?;
    // Given content painted once, then two WebSocket viewers and one local viewer.
    fx.publish(b"\x1b]0;build\x07\x1b[?2004h\x1b[1;32mready\x1b[0m\r\n$ make\r\n")?;
    let mut tui = attach(&fx, ("owner", Transport::Ws), "w-1").await?;
    let mut phone = attach(&fx, ("phone", Transport::Ws), "w-2").await?;
    let mut local = attach(&fx, ("bridge", Transport::Unix), "w-3").await?;
    assert_eq!(at(&tui.result, "/result/lease/role"), "interact");
    let paint = at(&tui.result, "/result/first_paint");
    assert_eq!(paint, at(&phone.result, "/result/first_paint"));
    assert_eq!(paint, at(&local.result, "/result/first_paint"));
    // When output arrives, a query split across reads among it.
    for chunk in [&b"compile 1\x1b[6"[..], b"n compiled\r\n", LAST_CHUNK] {
        fx.publish(chunk)?;
    }
    for viewer in [&mut tui, &mut phone, &mut local] {
        viewer
            .until(|viewer| viewer.received.ends_with(LAST_CHUNK))
            .await?;
    }
    // Then a replay at the same stream position answers the same checkpoint on both
    // transports.
    let (tui_replay, phone_replay) = (repaint(&mut tui).await?, repaint(&mut phone).await?);
    let local_replay = repaint(&mut local).await?;
    assert_eq!(tui_replay, local_replay, "WebSocket and local replay agree");
    assert_eq!(phone_replay, local_replay);
    // When the phone loses its WebSocket and reconnects, it attaches afresh at the head.
    let old_stream = phone.stream.clone();
    drop(phone);
    let mut again = attach(&fx, ("phone", Transport::Ws), "w-4").await?;
    assert_ne!(
        again.stream, old_stream,
        "a reconnect never reuses a stream identity"
    );
    let head = fx.raw.len().to_string();
    assert_eq!(
        at(&again.result, "/result/first_paint/output_offset"),
        head.as_str()
    );
    fx.publish(b"after reconnect\r\n")?;
    fx.exit()?;
    let raw_end = fx.raw.len().to_string();
    for viewer in [&mut tui, &mut again, &mut local] {
        closes_exited(viewer, &raw_end).await?;
        matches_continuous_vt(viewer, &fx.raw)?;
    }
    // Then the WebSocket and local viewers received the same bytes in the same order.
    assert_eq!(tui.received, local.received);
    let report = fx.close().await?;
    assert!(report.failures().is_empty(), "{report:?}");
    Ok(())
}
