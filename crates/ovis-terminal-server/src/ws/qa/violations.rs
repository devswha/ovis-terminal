//! WebSocket-only violations close with an RFC 6455 code and the typed kind.
//!
//! Requests inside a violating message are never answered. Browser origins are checked
//! before the upgrade, and a peer that never upgrades is cut off.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpStream;

use super::TestResult;
use super::client::{BINARY, PING, Received, TEXT, WsClient, masked};
use super::fixture::{Fixture, ORIGIN, Transport};
use super::socket::{Client, at, request};

const WAIT: Duration = Duration::from_secs(15);

async fn upgraded(fx: &Fixture) -> TestResult<WsClient> {
    match fx.connect("owner", Transport::Ws).await? {
        Client::Ws(socket) => Ok(socket),
        Client::Unix(_) => Err("not a WebSocket client".into()),
    }
}

/// Sends `bytes` after hello and expects the close frame `(code, reason)`, then the end.
async fn closes_with(fx: &Fixture, bytes: &[u8], (code, reason): (u16, &str)) -> TestResult {
    let mut socket = upgraded(fx).await?;
    socket.send_raw(bytes).await?;
    assert_eq!(
        socket.next().await?,
        Some(Received::Close(Some(code), reason.to_owned()))
    );
    assert_eq!(
        socket.next().await?,
        None,
        "nothing follows the close frame"
    );
    Ok(())
}

fn credit_probe(id: &str) -> TestResult<Vec<u8>> {
    let probe = json!({"stream_id": "stream:none", "acknowledged": "0"});
    let mut line = serde_json::to_vec(&request(id, "attach.credit", &probe))?;
    line.push(b'\n');
    Ok(line)
}

/// A request split over a fragmented binary message with a ping between its fragments
/// is answered once; the ping is answered on the transport.
async fn fragments_are_one_stream(fx: &Fixture) -> TestResult {
    let mut socket = upgraded(fx).await?;
    let line = credit_probe("split")?;
    let (head, tail) = line.split_at(7);
    let frames = [
        masked(BINARY, head)?,
        masked(0x80 | PING, b"p")?,
        masked(0x80, tail)?,
    ];
    socket.send_raw(&frames.concat()).await?;
    assert_eq!(socket.next().await?, Some(Received::Pong(b"p".to_vec())));
    let mut client = Client::Ws(socket);
    let reply = client.expect_next().await?;
    assert_eq!(at(&reply, "/id"), "split");
    Ok(())
}

async fn origins_are_checked_before_the_upgrade(fx: &Fixture) -> TestResult {
    let refused = WsClient::connect(fx.ws_address, Some("https://evil.example"))
        .await?
        .err()
        .ok_or("a foreign origin was upgraded")?;
    assert!(refused.starts_with("HTTP/1.1 403 "), "{refused}");
    let body: Value = serde_json::from_str(refused.split("\r\n\r\n").nth(1).unwrap_or_default())?;
    assert_eq!(at(&body, "/kind"), "origin_rejected");
    assert!(
        !refused.contains("evil.example"),
        "the refusal echoes nothing"
    );
    let allowed = WsClient::connect(fx.ws_address, Some(ORIGIN))
        .await?
        .map_err(|response| format!("configured origin refused: {response}"))?;
    let reply = Client::Ws(allowed).hello(&fx.bearer("owner")?).await?;
    assert!(reply.get("result").is_some(), "{reply}");
    Ok(())
}

async fn a_silent_peer_is_cut_off(fx: &Fixture) -> TestResult {
    let idle = fx.idle_ws(Duration::from_millis(100)).await?;
    let mut silent = TcpStream::connect(idle.local_addr()).await?;
    let mut rest = Vec::new();
    tokio::time::timeout(WAIT, silent.read_to_end(&mut rest)).await??;
    assert!(rest.is_empty(), "a timed-out upgrade gets no response");
    let report = idle.shutdown().await?;
    assert_eq!(
        report.failures().get("handshake_timeout"),
        Some(&1),
        "{report:?}"
    );
    Ok(())
}

pub(crate) async fn violations() -> TestResult {
    let fx = Fixture::new().await?;
    // A request sent as a text message is never answered.
    let text = masked(0x80 | TEXT, &credit_probe("text")?)?;
    closes_with(&fx, &text, (1003, "invalid_ws_message")).await?;
    closes_with(&fx, &[0x82, 0x01, b'x'], (1002, "ws_protocol")).await?;
    // A declared length beyond one maximal frame is refused before any payload arrives.
    let declared = [0x82, 0xFF, 0, 0, 0, 0, 0x01, 0, 0, 0x0A, 1, 2, 3, 4];
    closes_with(&fx, &declared, (1009, "message_too_large")).await?;
    fragments_are_one_stream(&fx).await?;
    origins_are_checked_before_the_upgrade(&fx).await?;
    a_silent_peer_is_cut_off(&fx).await?;
    let report = fx.close().await?;
    for kind in [
        "invalid_ws_message",
        "ws_protocol",
        "message_too_large",
        "origin_rejected",
    ] {
        assert_eq!(report.failures().get(kind), Some(&1), "{kind}: {report:?}");
    }
    Ok(())
}
