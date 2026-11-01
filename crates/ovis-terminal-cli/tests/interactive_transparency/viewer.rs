//! A reference viewer that speaks the protocol directly.
//!
//! It observes a terminal on its own connection and keeps exactly the bytes the server put
//! in `attach.data`, so the attach client's output can be compared with them byte for byte.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use serde_json::{Value, json};

use crate::support::{Served, TestResult, WAIT};

type Chunk = (String, Vec<u8>);

/// One observe stream read on its own connection.
#[derive(Debug)]
pub struct Viewer {
    stream: UnixStream,
    id: String,
    data: mpsc::Receiver<Chunk>,
    /// Every `attach.data` byte of this stream received so far, in order.
    pub received: Vec<u8>,
}

/// Routes every line from the server: replies by id, and every `attach.data` as its stream
/// and bytes, including data that arrives before the `attach.open` reply.
fn route(
    lines: BufReader<UnixStream>,
    (replies, data): (mpsc::Sender<Value>, mpsc::Sender<Chunk>),
) {
    for line in lines.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            break;
        };
        if value.get("id").is_some() {
            // The receiver is gone only once the test stopped awaiting replies.
            match replies.send(value) {
                Ok(()) | Err(_) => {}
            }
            continue;
        }
        if value.get("method").and_then(Value::as_str) != Some("attach.data") {
            continue;
        }
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let stream = params.get("stream_id").and_then(Value::as_str);
        let encoded = params.get("data_b64").and_then(Value::as_str);
        let Ok(bytes) =
            base64::engine::general_purpose::STANDARD.decode(encoded.unwrap_or_default())
        else {
            break;
        };
        if data
            .send((stream.unwrap_or_default().to_owned(), bytes))
            .is_err()
        {
            break;
        }
    }
}

/// Sends one request and returns the result of its reply, the next reply on the connection.
fn call(
    stream: &mut UnixStream,
    replies: &mpsc::Receiver<Value>,
    (id, method, params): (&str, &str, Value),
) -> TestResult<Value> {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    stream.write_all(format!("{line}\n").as_bytes())?;
    let reply = replies.recv_timeout(WAIT)?;
    if reply.get("id").and_then(Value::as_str) != Some(id) {
        return Err(format!("expected the {id} reply, got {reply}").into());
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{method} failed: {reply}").into())
}

impl Viewer {
    /// Opens an observe stream on `address` under the client label `client`.
    ///
    /// # Errors
    ///
    /// Fails when the connection, hello or `attach.open` fails.
    pub fn observe(served: &Served, (address, client): (&str, &str)) -> TestResult<Self> {
        let (socket, bearer) = served.endpoint()?;
        let mut stream = UnixStream::connect(socket)?;
        let lines = BufReader::new(stream.try_clone()?);
        let (reply_sender, replies) = mpsc::channel();
        let (data_sender, data) = mpsc::channel();
        std::thread::spawn(move || route(lines, (reply_sender, data_sender)));
        let hello = json!({
            "generation": PROTOCOL_GENERATION,
            "client": {"name": "reference-viewer", "version": "0"},
            "credential": bearer,
        });
        call(&mut stream, &replies, ("hello", "hello", hello))?;
        let open = json!({
            "op_id": uuid::Uuid::now_v7().to_string(), "target": {"address": address},
            "role": "observe", "client_id": client, "geometry": null,
            "stream_mode": "plain", "takeover": null,
        });
        let opened = call(&mut stream, &replies, ("open", "attach.open", open))?;
        let id = opened.get("stream_id").and_then(Value::as_str);
        Ok(Self {
            stream,
            id: id
                .ok_or_else(|| format!("no stream_id in {opened}"))?
                .to_owned(),
            data,
            received: Vec::new(),
        })
    }

    /// Waits, bounded, until `needle` is among this stream's received bytes.
    pub fn until(&mut self, needle: &[u8], within: Duration) -> bool {
        let Some(deadline) = Instant::now().checked_add(within) else {
            return false;
        };
        while !self.received.windows(needle.len()).any(|w| w == needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.data.recv_timeout(left) {
                Ok((stream, chunk)) if stream == self.id => self.received.extend(chunk),
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        true
    }

    /// Closes the connection, which ends the stream and its reader.
    ///
    /// # Errors
    ///
    /// Fails when the socket cannot be shut down.
    pub fn close(self) -> TestResult {
        Ok(self.stream.shutdown(std::net::Shutdown::Both)?)
    }
}
