//! One NDJSON client over either transport, so every scenario step runs unchanged over
//! the local socket and over WebSocket.

use std::io::Write as _;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

use super::TestResult;
use super::client::{BINARY, Received, WsClient};

const WAIT: Duration = Duration::from_secs(15);

pub(crate) fn request(id: &str, method: &str, params: &Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

pub(crate) fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

pub(crate) fn error_kind(reply: &Value) -> &str {
    at(reply, "/error/data/kind").as_str().unwrap_or_default()
}

#[derive(Debug)]
pub(crate) enum Client {
    Unix(BufReader<UnixStream>),
    Ws(WsClient),
}

impl Client {
    pub(crate) async fn send(&mut self, message: &Value) -> TestResult {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        match self {
            Self::Unix(stream) => stream.get_mut().write_all(&line).await?,
            Self::Ws(socket) => socket.send(BINARY, &line).await?,
        }
        if message.get("method") != Some(&json!("hello")) {
            writeln!(std::io::stdout(), "-> {message}")?;
        }
        Ok(())
    }

    pub(crate) async fn hello(&mut self, bearer: &str) -> TestResult<Value> {
        let params = json!({
            "generation": "1", "client": {"name": "ws-test", "version": "1"},
            "credential": bearer,
        });
        self.send(&request("hello", "hello", &params)).await?;
        self.expect_next().await
    }

    /// The next record. Over WebSocket every record must be exactly one binary message
    /// holding one NDJSON line.
    pub(crate) async fn expect_next(&mut self) -> TestResult<Value> {
        let line = match self {
            Self::Unix(stream) => {
                let mut line = String::new();
                let read = tokio::time::timeout(WAIT, stream.read_line(&mut line)).await??;
                if read == 0 {
                    return Err("connection ended before reply".into());
                }
                line
            }
            Self::Ws(socket) => match socket.next().await? {
                Some(Received::Binary(message)) => {
                    let line = String::from_utf8(message)?;
                    let one_line = line.ends_with('\n') && line.matches('\n').count() == 1;
                    if !one_line {
                        return Err(format!("not one record per message: {line:?}").into());
                    }
                    line
                }
                other => return Err(format!("expected a record, got {other:?}").into()),
            },
        };
        writeln!(std::io::stdout(), "<- {}", line.trim_end())?;
        Ok(serde_json::from_str(&line)?)
    }
}
