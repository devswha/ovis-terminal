//! A scripted machine at the far end of an in-process line: it records every request it
//! reads, then answers it, refuses it or leaves it pending.

use std::sync::{Arc, Mutex, PoisonError};

use ovis_terminal_remote::actors::Lines;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::{HANG, TestResult};

pub(super) type Pipe = Lines<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

/// How the machine treats each request.
#[derive(Debug, Clone, Copy)]
pub(super) enum Mode {
    /// Answers with the method and parameters it received.
    Echo,
    /// Refuses with its own typed `spawn_failed`.
    Refuse,
    /// Never answers.
    Silent,
}

type Writer = Arc<tokio::sync::Mutex<Option<WriteHalf<DuplexStream>>>>;

#[derive(Debug)]
pub(super) struct Peer {
    received: Arc<Mutex<Vec<Value>>>,
    count: watch::Receiver<usize>,
    writer: Writer,
    task: JoinHandle<()>,
}

async fn write(writer: &Writer, record: &Value) -> TestResult {
    let mut guard = writer.lock().await;
    let half = guard.as_mut().ok_or("the line was cut")?;
    half.write_all(format!("{record}\n").as_bytes()).await?;
    drop(guard);
    Ok(())
}

fn answer(mode: Mode, request: &Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    match mode {
        Mode::Echo => Some(json!({"jsonrpc": "2.0", "id": id, "result": {
            "method": request.get("method"), "params": request.get("params"),
        }})),
        Mode::Refuse => Some(json!({"jsonrpc": "2.0", "id": id, "error": {
            "code": -32000, "message": "spawn_failed",
            "data": {"kind": "spawn_failed", "hint": "fix the program", "retryable": false},
        }})),
        Mode::Silent => None,
    }
}

impl Peer {
    /// Serves `mode` on a fresh line; the returned line belongs to the local actor.
    pub(super) fn start(mode: Mode) -> (Self, Pipe) {
        let (local, remote) = tokio::io::duplex(64 * 1024);
        let (local_read, local_write) = tokio::io::split(local);
        let (remote_read, remote_write) = tokio::io::split(remote);
        let writer: Writer = Arc::new(tokio::sync::Mutex::new(Some(remote_write)));
        let received = Arc::new(Mutex::new(Vec::new()));
        let (counted, count) = watch::channel(0);
        let task = tokio::spawn({
            let (writer, received) = (Arc::clone(&writer), Arc::clone(&received));
            async move {
                let mut reader = BufReader::new(remote_read);
                let mut record = String::new();
                while matches!(reader.read_line(&mut record).await, Ok(1..)) {
                    let request: Value = serde_json::from_str(&record).unwrap_or(Value::Null);
                    record.clear();
                    if request.get("id").is_none() {
                        continue;
                    }
                    let reply = answer(mode, &request);
                    let total = {
                        let mut seen = received.lock().unwrap_or_else(PoisonError::into_inner);
                        seen.push(request);
                        seen.len()
                    };
                    counted.send_replace(total);
                    if let Some(reply) = reply
                        && write(&writer, &reply).await.is_err()
                    {
                        break;
                    }
                }
            }
        });
        let peer = Self {
            received,
            count,
            writer,
            task,
        };
        (peer, Lines::new(local_read, local_write))
    }

    /// Every request read so far, in order.
    pub(super) fn received(&self) -> Vec<Value> {
        self.received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits, bounded, until `total` requests were read.
    pub(super) async fn read(&mut self, total: usize) -> TestResult {
        tokio::time::timeout(HANG, self.count.wait_for(|seen| *seen >= total)).await??;
        Ok(())
    }

    /// Sends an `event` notification carrying remote sequence `seq`.
    pub(super) async fn event(&self, seq: &str) -> TestResult {
        let event = json!({"jsonrpc": "2.0", "method": "event", "params": {
            "seq": seq, "runtime_lifetime": "7", "ts_ms": "1", "type": "terminal.created",
            "terminal": null, "data": {},
        }});
        write(&self.writer, &event).await
    }

    /// Ends the line: the machine stops reading and its end is dropped.
    pub(super) async fn cut(&self) {
        self.task.abort();
        drop(self.writer.lock().await.take());
    }
}
