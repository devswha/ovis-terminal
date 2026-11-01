//! The attached client against a scripted peer on a real Unix socket: readiness changes
//! (including the server's `$/closing`) decide when typed bytes reach `input.raw`, and only
//! a CLOSE the server sent counts as a confirmed detach.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use base64::Engine as _;
use ovis_terminal_protocol::schema::ids::{ClientId, StreamId};
use ovis_terminal_protocol::schema::records::{AttachControl, ControlKind};
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixListener;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc;

use super::super::AttachPlan;
use super::super::client::{Client, Notice};
use super::super::screen::Screen;
use super::Attached;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type PeerResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const STREAM: &str = "stream:1";

/// What the peer does once it answered `attach.close`.
#[derive(Debug, Clone, Copy)]
enum AfterClose {
    /// Sends the stream's CLOSE.
    Confirm,
    /// Hangs up without a CLOSE.
    HangUp,
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) | Err(_) => {}
        }
    }
}

fn scratch() -> TestResult<Scratch> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("ovt31s-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&path)?;
    Ok(Scratch(path))
}

async fn send(stream: &mut OwnedWriteHalf, record: &Value) -> PeerResult {
    let mut line = record.to_string().into_bytes();
    line.push(b'\n');
    stream.write_all(&line).await?;
    Ok(())
}

fn control(kind: ControlKind) -> PeerResult<Value> {
    let record = AttachControl::new(StreamId::new(STREAM.to_owned()), kind, ByteOffset::new(0));
    Ok(
        json!({"jsonrpc": "2.0", "method": "attach.control", "params": serde_json::to_value(record)?}),
    )
}

/// Answers every request; reports the decoded bytes of each `input.raw` on `inputs`.
async fn peer(
    listener: UnixListener,
    after_close: AfterClose,
    inputs: mpsc::UnboundedSender<Vec<u8>>,
) -> PeerResult {
    let (stream, _) = listener.accept().await?;
    let (read, mut stream) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let request: Value = serde_json::from_str(&line)?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "input.raw" {
            let data = request.pointer("/params/data_b64").and_then(Value::as_str);
            let bytes =
                base64::engine::general_purpose::STANDARD.decode(data.unwrap_or_default())?;
            inputs.send(bytes)?;
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        send(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        )
        .await?;
        if method == "attach.close" {
            match after_close {
                AfterClose::Confirm => send(&mut stream, &control(ControlKind::Close)?).await?,
                AfterClose::HangUp => return Ok(()),
            }
        }
    }
    Ok(())
}

async fn connected(
    after_close: AfterClose,
) -> TestResult<(
    Attached,
    mpsc::UnboundedReceiver<Notice>,
    mpsc::UnboundedReceiver<Vec<u8>>,
    Scratch,
)> {
    let dir = scratch()?;
    let socket = dir.0.join("s.sock");
    let listener = UnixListener::bind(&socket)?;
    let (inputs, received) = mpsc::unbounded_channel();
    tokio::spawn(peer(listener, after_close, inputs));
    let (client, notices) = Client::connect(&socket, None).await?;
    let plan = AttachPlan {
        target: serde_json::from_value(json!({"address": "t1"}))?,
        observe: false,
        client: ClientId::new("cli".to_owned()),
        takeover: None,
    };
    let attached = Attached {
        client,
        plan,
        stream: STREAM.to_owned(),
        lease: Some("lease:1".to_owned()),
        geometry_lease: None,
        displaced: Value::Null,
        window: 1 << 20,
        acknowledged: 0,
        input_ready: false,
        held: Vec::new(),
        screen: Screen::open(false)?,
        closing: None,
    };
    Ok((attached, notices, received, dir))
}

fn notice(kind: ControlKind) -> TestResult<Notice> {
    let record = control(kind).map_err(|error| error.to_string())?;
    let params = record.get("params").cloned().ok_or("params")?;
    Ok(("attach.control".to_owned(), params))
}

fn runtime() -> TestResult<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

#[test]
fn input_revoked_holds_typed_bytes_until_the_stream_is_ready_again() -> TestResult {
    runtime()?.block_on(async {
        let (mut attached, _notices, mut received, _dir) = connected(AfterClose::Confirm).await?;
        attached.notice(notice(ControlKind::InputReady)?).await?;
        attached.typed(b"a").await?;
        assert_eq!(received.try_recv().ok(), Some(b"a".to_vec()));
        attached.notice(notice(ControlKind::InputRevoked)?).await?;
        attached.typed(b"b").await?;
        assert!(
            received.try_recv().is_err(),
            "input reached input.raw after input_revoked"
        );
        assert_eq!(attached.held, b"b".to_vec());
        attached.notice(notice(ControlKind::InputReady)?).await?;
        assert_eq!(received.try_recv().ok(), Some(b"b".to_vec()));
        assert_eq!(attached.held, Vec::<u8>::new());
        Ok(())
    })
}

#[test]
fn a_detach_is_confirmed_only_by_the_streams_close() -> TestResult {
    runtime()?.block_on(async {
        let (mut attached, mut notices, _received, _dir) = connected(AfterClose::HangUp).await?;
        let unconfirmed = attached.detach(&mut notices, "detach_key").await;
        assert_eq!(
            unconfirmed.pointer("/detached/stream_closed"),
            Some(&json!(false)),
            "{unconfirmed}"
        );
        let (mut attached, mut notices, _received, _dir) = connected(AfterClose::Confirm).await?;
        let confirmed = attached.detach(&mut notices, "detach_key").await;
        assert_eq!(
            confirmed.pointer("/detached/stream_closed"),
            Some(&json!(true))
        );
        Ok(())
    })
}

#[test]
fn a_closing_record_revokes_readiness_and_is_kept_for_the_loss_report() -> TestResult {
    runtime()?.block_on(async {
        let (mut attached, _notices, mut received, _dir) = connected(AfterClose::Confirm).await?;
        attached.notice(notice(ControlKind::InputReady)?).await?;
        attached.typed(b"a").await?;
        assert_eq!(received.try_recv().ok(), Some(b"a".to_vec()));
        let closing = json!({"reason": "server_shutdown", "hint": "reconnect later"});
        attached
            .notice(("$/closing".to_owned(), closing.clone()))
            .await?;
        attached.typed(b"b").await?;
        assert!(
            received.try_recv().is_err(),
            "input reached input.raw after $/closing"
        );
        assert_eq!(attached.held, b"b".to_vec());
        assert_eq!(attached.closing, Some(closing));
        Ok(())
    })
}
