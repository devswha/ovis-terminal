//! Output credit against a scripted peer that honours it as the server does: it sends
//! `attach.data` only while the unacknowledged bytes fit the window and sends more only
//! after `attach.credit`. A download of several windows completes only when the client
//! replenishes credit; without replenishment the peer stops and the bounded wait fails.

use std::time::Duration;

use base64::Engine as _;
use ovis_terminal_protocol::schema::ids::{ClientId, StreamId};
use ovis_terminal_protocol::schema::records::AttachData;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::net::UnixListener;

use super::super::AttachPlan;
use super::super::client::Client;
use super::super::screen::Screen;
use super::Attached;
use super::tests::{PeerResult, STREAM, TestResult, runtime, scratch, send};

/// Credit the client starts with, as `attach.open` would grant it.
const WINDOW: u64 = 16;
/// Bytes per `attach.data` notification.
const CHUNK: u64 = 4;
/// The whole download: four windows.
const TOTAL: u64 = 64;
/// Bound on the download completing; a stalled stream never does.
const BOUND: Duration = Duration::from_secs(10);

fn data(offset: u64) -> PeerResult<Value> {
    let bytes = vec![b'.'; usize::try_from(CHUNK)?];
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let record = AttachData::new(
        StreamId::new(STREAM.to_owned()),
        ByteOffset::new(offset),
        encoded,
    );
    Ok(json!({"jsonrpc": "2.0", "method": "attach.data", "params": serde_json::to_value(record)?}))
}

/// Sends within credit, takes cumulative acknowledgements from `attach.credit`.
async fn credited_peer(listener: UnixListener) -> PeerResult {
    let (stream, _) = listener.accept().await?;
    let (read, mut stream) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let (mut sent, mut acknowledged) = (0_u64, 0_u64);
    loop {
        while sent < TOTAL && sent.saturating_sub(acknowledged) < WINDOW {
            send(&mut stream, &data(sent)?).await?;
            sent = sent.saturating_add(CHUNK);
        }
        let Some(line) = lines.next_line().await? else {
            return Ok(());
        };
        let request: Value = serde_json::from_str(&line)?;
        if request.get("method").and_then(Value::as_str) == Some("attach.credit") {
            let at = request
                .pointer("/params/acknowledged")
                .and_then(Value::as_str);
            acknowledged = at.ok_or("acknowledged")?.parse()?;
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let reply = json!({"jsonrpc": "2.0", "id": id, "result": {}});
        send(&mut stream, &reply).await?;
    }
}

#[test]
fn a_download_larger_than_the_window_completes_only_through_replenished_credit() -> TestResult {
    runtime()?.block_on(async {
        let dir = scratch()?;
        let socket = dir.0.join("s.sock");
        let listener = UnixListener::bind(&socket)?;
        tokio::spawn(credited_peer(listener));
        let (client, mut notices) = Client::connect(&socket, None).await?;
        let plan = AttachPlan {
            target: serde_json::from_value(json!({"address": "t1"}))?,
            observe: true,
            client: ClientId::new("cli".to_owned()),
            takeover: None,
        };
        let mut attached = Attached {
            client,
            plan,
            stream: STREAM.to_owned(),
            lease: None,
            geometry_lease: None,
            displaced: Value::Null,
            window: WINDOW,
            acknowledged: 0,
            input_ready: false,
            held: Vec::new(),
            screen: Screen::open(false)?,
            closing: None,
        };
        let download = async {
            while attached.acknowledged < TOTAL {
                let Some(notice) = notices.recv().await else {
                    return Err("the peer hung up before the download completed".into());
                };
                attached.notice(notice).await?;
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        };
        let completed = tokio::time::timeout(BOUND, download).await;
        assert!(
            completed.is_ok(),
            "the stream stalled at {} of {TOTAL} bytes: credit was not replenished",
            attached.acknowledged
        );
        completed.map_err(|_| "unreachable")??;
        assert_eq!(attached.acknowledged, TOTAL);
        Ok(())
    })
}
