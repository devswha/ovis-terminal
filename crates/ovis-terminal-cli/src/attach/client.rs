//! One persistent NDJSON connection carrying correlated requests and the stream's
//! notifications; a reader task routes replies by id and notifications in order.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ovis_terminal_protocol::rpc::{RpcErrorObject, RpcReply, parse_reply};
use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};

use super::AttachError;

type Settled = Result<Value, Box<RpcErrorObject>>;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Settled>>>>;

/// A server notification: method and params.
pub(super) type Notice = (String, Value);

/// An authenticated connection.
#[derive(Debug)]
pub(super) struct Client {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    next: AtomicU64,
}

fn io(operation: &'static str) -> impl FnOnce(std::io::Error) -> AttachError {
    move |source| AttachError::Io { operation, source }
}

async fn read(read: OwnedReadHalf, pending: Pending, notices: mpsc::UnboundedSender<Notice>) {
    let limit = u64::try_from(MAX_FRAME_PAYLOAD).unwrap_or(u64::MAX);
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        match (&mut reader).take(limit).read_line(&mut line).await {
            Ok(read) if read > 0 && line.ends_with('\n') => {}
            Ok(_) | Err(_) => break,
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            break;
        };
        let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
        match (id, value.get("method").and_then(Value::as_str)) {
            (Some(id), None) => {
                let waiter = pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                let settled = match parse_reply(&line) {
                    Ok(RpcReply::Success(success)) => Ok(success.result),
                    Ok(RpcReply::Failure(failure)) => Err(Box::new(failure.error)),
                    Ok(_) | Err(_) => break,
                };
                if let Some(waiter) = waiter {
                    match waiter.send(settled) {
                        Ok(()) | Err(_) => {}
                    }
                }
            }
            (None, Some(method)) => {
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                if notices.send((method.to_owned(), params)).is_err() {
                    break;
                }
            }
            _ => break,
        }
    }
    pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

impl Client {
    /// Connects to `path` and completes hello with `bearer`; notifications arrive on the
    /// returned receiver in server order.
    pub(super) async fn connect(
        path: &Path,
        bearer: Option<&str>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Notice>), AttachError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(io("connect to the endpoint"))?;
        let (reader, writer) = stream.into_split();
        let pending: Pending = Arc::default();
        let (sender, notices) = mpsc::unbounded_channel();
        tokio::spawn(read(reader, Arc::clone(&pending), sender));
        let client = Self {
            writer: tokio::sync::Mutex::new(writer),
            pending,
            next: AtomicU64::new(1),
        };
        let mut hello = json!({
            "generation": PROTOCOL_GENERATION,
            "client": {"name": crate::BINARY_NAME, "version": crate::VERSION},
        });
        if let (Some(bearer), Value::Object(fields)) = (bearer, &mut hello) {
            fields.insert("credential".to_owned(), json!(bearer));
        }
        client.call("hello", hello).await?;
        Ok((client, notices))
    }

    /// Sends `method` and waits for its reply; a typed server error stays typed.
    pub(super) async fn call(&self, method: &str, params: Value) -> Result<Value, AttachError> {
        let id = format!("attach-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (waiter, settled) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), waiter);
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = request.to_string().into_bytes();
        line.push(b'\n');
        self.writer
            .lock()
            .await
            .write_all(&line)
            .await
            .map_err(|_| AttachError::ConnectionLost)?;
        settled
            .await
            .map_err(|_| AttachError::ConnectionLost)?
            .map_err(AttachError::Server)
    }
}
