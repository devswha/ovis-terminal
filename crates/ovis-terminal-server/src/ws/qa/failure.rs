//! Authority over WebSocket is the local socket's.
//!
//! An expired credential is refused at hello and nothing attaches after it, an observer
//! cannot take input, and a client DATA frame on a stream channel closes the connection
//! with the same typed reason. No reply carries the presented credential.

use ovis_terminal_authz::Permission;
use ovis_terminal_protocol::wire::{
    Channel, ConnectionDecoder, FeedOutcome, Frame, FrameKind, WireMessage,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::TestResult;
use super::client::{BINARY, Received};
use super::fixture::{Fixture, Transport};
use super::socket::{Client, at, error_kind, request};
use super::viewer::{Viewer, open_params};

const OBSERVER: &[Permission] = &[Permission::Observe];
/// Long before any real clock reading: the credential is expired at every hello.
const EXPIRED_AT: u64 = 2_000;
const BOTH: [Transport; 2] = [Transport::Ws, Transport::Unix];

/// hello with an expired credential, then an attach on the same connection.
async fn expired_then_attach(fx: &Fixture, transport: Transport) -> TestResult<(Value, Value)> {
    let bearer = fx.bearer("late")?;
    let mut client = fx.open(transport).await?;
    let hello = client.hello(&bearer).await?;
    client
        .send(&request(
            "attach",
            "attach.open",
            &open_params("f-0", "late", "observe"),
        ))
        .await?;
    let attach = client.expect_next().await?;
    for reply in [&hello, &attach] {
        assert!(
            !reply.to_string().contains(&bearer),
            "credentials are never echoed"
        );
    }
    Ok((hello, attach))
}

/// After a framed hello, a client DATA frame on stream channel 1; returns `$/closing`.
async fn upstream_data(fx: &Fixture, transport: Transport) -> TestResult<Value> {
    let mut client = fx.open(transport).await?;
    let hello = json!({
        "generation": "1", "client": {"name": "ws-test", "version": "1"},
        "framed": true, "credential": fx.bearer("phone")?,
    });
    client.send(&request("hello", "hello", &hello)).await?;
    assert!(client.expect_next().await?.get("result").is_some());
    let data = Frame::new(Channel::new(1), FrameKind::Data, b"\x03rm -rf ~\r".to_vec())?;
    let mut decoder = ConnectionDecoder::new();
    decoder.upgrade_to_framed()?;
    let message = match &mut client {
        Client::Unix(stream) => {
            stream.get_mut().write_all(&data.encode()?).await?;
            let mut written = Vec::new();
            let until_closed = stream.read_to_end(&mut written);
            tokio::time::timeout(std::time::Duration::from_secs(15), until_closed).await??;
            written
        }
        Client::Ws(socket) => {
            socket.send(BINARY, &data.encode()?).await?;
            let Some(Received::Binary(message)) = socket.next().await? else {
                return Err("expected one framed record".into());
            };
            assert_eq!(
                socket.next().await?,
                Some(Received::Close(Some(1000), String::new()))
            );
            assert_eq!(socket.next().await?, None, "the server ends the TCP stream");
            message
        }
    };
    let consumed = decoder.feed(&message)?;
    assert!(
        matches!(consumed, FeedOutcome::Accepted { .. }),
        "{consumed:?}"
    );
    let Some(WireMessage::Frame(frame)) = decoder.next_message()? else {
        return Err("expected a control frame".into());
    };
    assert!(
        decoder.next_message()?.is_none(),
        "exactly one record, then the end"
    );
    assert_eq!(frame.channel, Channel::CONTROL);
    Ok(serde_json::from_slice(&frame.payload)?)
}

pub(crate) async fn failure() -> TestResult {
    let fx = Fixture::new().await?;
    fx.issue("late", (OBSERVER, EXPIRED_AT))?;
    fx.issue("phone", (OBSERVER, u64::MAX))?;
    // An expired credential is refused before anything attaches, as on the local socket.
    let (ws_hello, ws_attach) = expired_then_attach(&fx, Transport::Ws).await?;
    let (unix_hello, unix_attach) = expired_then_attach(&fx, Transport::Unix).await?;
    assert_eq!(error_kind(&ws_hello), "credential_expired", "{ws_hello}");
    assert_eq!(error_kind(&ws_attach), "unauthenticated", "{ws_attach}");
    assert_eq!((ws_hello, ws_attach), (unix_hello, unix_attach));
    // An observe-only credential asking to interact is refused, with no stream.
    let mut refused = Vec::new();
    for (transport, op) in BOTH.into_iter().zip(["f-1", "f-2"]) {
        let client = fx.connect("phone", transport).await?;
        let viewer = Viewer::open(client, &open_params(op, "phone", "interact"), true).await?;
        assert_eq!(at(&viewer.result, "/result/outcome"), "refused");
        assert_eq!(at(&viewer.result, "/result/reason"), "forbidden");
        assert!(at(&viewer.result, "/result/stream_id").is_null());
        assert!(
            at(&viewer.result, "/result/lease").is_null(),
            "no lease, so no input path"
        );
        refused.push(at(&viewer.result, "/result").clone());
    }
    assert_eq!(refused.first(), refused.last());
    // A client DATA frame on a stream channel closes both transports the same way.
    let ws_closing = upstream_data(&fx, Transport::Ws).await?;
    let unix_closing = upstream_data(&fx, Transport::Unix).await?;
    assert_eq!(at(&ws_closing, "/method"), "$/closing");
    assert_eq!(at(&ws_closing, "/params/reason"), "unowned_channel");
    assert_eq!(ws_closing, unix_closing);
    let report = fx.close().await?;
    assert_eq!(
        report.failures().get("unowned_channel"),
        Some(&1),
        "{report:?}"
    );
    Ok(())
}
