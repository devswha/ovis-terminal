//! Lines over in-process byte pipes whose remote end each test drives record by record,
//! so stalls and losses are real stream states rather than mocked outcomes.

use std::time::Duration;

use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::MachineRecord;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf,
};

use crate::actors::{Actors, Inbound, Limits, Lines};

/// Result of a test.
pub type TestResult = Result<(), Box<dyn std::error::Error>>;
/// A line over an in-process pipe.
pub type PipeLine = Lines<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

/// Bound on any single wait in these tests; never reached when the behavior holds.
pub const HANG: Duration = Duration::from_secs(10);

/// The machine id ending in `last`.
pub fn machine(last: u8) -> MachineId {
    MachineId::new(format!(
        "machine:0191f1a2-0000-7000-8000-0000000000{last:02x}"
    ))
}

/// A connected machine record.
///
/// # Errors
///
/// The pipe, decoding or actor failure.
pub fn record(last: u8, name: &str) -> Result<MachineRecord, serde_json::Error> {
    serde_json::from_value(json!({
        "machine_id": machine(last), "name": name, "aliases": [], "state": "connected",
        "remote_lifetime": "7", "remote_boot_id": "1", "last_seen_seq": null,
        "version": "1.0.0", "revision": "1",
    }))
}

/// The value at `pointer` in `value`, or null.
pub fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

/// The remote end of a line: reads the actor's records and writes its own.
#[derive(Debug)]
pub struct Peer {
    /// The read side of the remote end.
    pub reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl Peer {
    /// The next record the actor wrote.
    ///
    /// # Errors
    ///
    /// The pipe, decoding or actor failure.
    pub async fn next(&mut self) -> Result<Value, Box<dyn std::error::Error>> {
        let mut line = String::new();
        let read = tokio::time::timeout(HANG, self.reader.read_line(&mut line)).await??;
        if read == 0 {
            return Err("the actor closed the line".into());
        }
        Ok(serde_json::from_str(&line)?)
    }

    /// Writes one record to the actor.
    ///
    /// # Errors
    ///
    /// The pipe, decoding or actor failure.
    pub async fn write(&mut self, record: &Value) -> TestResult {
        let mut bytes = record.to_string().into_bytes();
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        Ok(())
    }

    /// Answers the next request with `result`, returning the request.
    ///
    /// # Errors
    ///
    /// The pipe, decoding or actor failure.
    pub async fn answer(&mut self, result: Value) -> Result<Value, Box<dyn std::error::Error>> {
        let request = self.next().await?;
        let reply = json!({"jsonrpc": "2.0", "id": at(&request, "/id"), "result": result});
        self.write(&reply).await?;
        Ok(request)
    }
}

/// A line whose pipes hold `capacity` bytes in each direction, and its remote end.
pub fn pipe(capacity: usize) -> (PipeLine, Peer) {
    let (local, remote) = tokio::io::duplex(capacity);
    let (local_read, local_write) = tokio::io::split(local);
    let (remote_read, remote_write) = tokio::io::split(remote);
    let peer = Peer {
        reader: BufReader::new(remote_read),
        writer: remote_write,
    };
    (Lines::new(local_read, local_write), peer)
}

/// Actors for `machines`, each attached to a fresh pipe whose remote end is returned.
///
/// # Errors
///
/// The pipe, decoding or actor failure.
pub async fn served(
    limits: Limits,
    machines: &[(u8, &str)],
) -> Result<
    (
        Actors<PipeLine>,
        tokio::sync::mpsc::Receiver<Inbound>,
        Vec<Peer>,
    ),
    Box<dyn std::error::Error>,
> {
    let (actors, inbound) = Actors::new(limits);
    let mut peers = Vec::new();
    for (last, name) in machines {
        actors.add(record(*last, name)?)?;
        let (line, peer) = pipe(64 * 1024);
        actors.attach(&machine(*last), line).await?;
        peers.push(peer);
    }
    Ok((actors, inbound, peers))
}

/// Bounds used by these tests.
pub const fn limits() -> Limits {
    Limits::new(8, Duration::from_secs(60), 64)
}
