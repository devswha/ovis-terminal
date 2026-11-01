//! One raw protocol connection of its own: hello with the server's bearer, then requests
//! answered in order. Notifications arriving before a reply are skipped.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;

use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use serde_json::{Value, json};

use crate::support::{Served, TestResult, WAIT};

/// A connection, and so a lease owner of its own, whatever client label it presents.
#[derive(Debug)]
pub struct Socket {
    stream: UnixStream,
    lines: BufReader<UnixStream>,
    next: u64,
}

impl Socket {
    /// Connects and completes hello with the server's owner bearer.
    ///
    /// # Errors
    ///
    /// Fails when the connection or hello fails.
    pub fn open(served: &Served) -> TestResult<Self> {
        let (socket, bearer) = served.endpoint()?;
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(WAIT))?;
        let lines = BufReader::new(stream.try_clone()?);
        let mut connection = Self {
            stream,
            lines,
            next: 0,
        };
        let hello = json!({
            "generation": PROTOCOL_GENERATION,
            "client": {"name": "connections-test", "version": "0"},
            "credential": bearer,
        });
        connection.result("hello", &hello)?;
        Ok(connection)
    }

    /// Sends `method` and returns its whole reply, `result` or `error`.
    ///
    /// # Errors
    ///
    /// Fails when the connection closes or stays silent past the bound.
    pub fn reply(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        self.next = self.next.saturating_add(1);
        let id = format!("{method}-{}", self.next);
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stream.write_all(format!("{line}\n").as_bytes())?;
        loop {
            let mut text = String::new();
            if self.lines.read_line(&mut text)? == 0 {
                return Err(format!("connection closed before the {id} reply").into());
            }
            let value: Value = serde_json::from_str(&text)?;
            if value.get("id").and_then(Value::as_str) == Some(id.as_str()) {
                return Ok(value);
            }
        }
    }

    /// Sends `method` and returns its result.
    ///
    /// # Errors
    ///
    /// Fails when the reply is a typed error or never arrives.
    pub fn result(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        let reply = self.reply(method, params)?;
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| format!("{method} failed: {reply}").into())
    }
}
