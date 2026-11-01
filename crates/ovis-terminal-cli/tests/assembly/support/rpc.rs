//! One JSON-RPC request on a raw connection, answered by id.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

use super::{Served, TestResult, op_id};

/// Sends one request on `connection` and returns its reply, skipping notifications.
///
/// # Errors
///
/// Fails when a socket step fails or the connection closes first.
pub fn rpc(
    connection: &mut BufReader<UnixStream>,
    method: &str,
    params: &Value,
) -> TestResult<Value> {
    let id = op_id()?;
    let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(connection.get_mut(), "{request}")?;
    loop {
        let mut line = String::new();
        if connection.read_line(&mut line)? == 0 {
            return Err("the connection closed".into());
        }
        let record: Value = serde_json::from_str(&line)?;
        if record.get("id") == Some(&json!(id)) {
            return Ok(record);
        }
    }
}

impl Served {
    /// A connection on the local socket that completed hello as the owner and stays open.
    ///
    /// # Errors
    ///
    /// Fails when a socket step fails or hello is refused.
    pub fn connected(&self) -> TestResult<BufReader<UnixStream>> {
        self.connected_as(&self.bearer)
    }

    /// A raw connection that completed hello with `bearer`.
    ///
    /// # Errors
    ///
    /// Fails when a socket step fails or hello is refused.
    pub fn connected_as(&self, bearer: &str) -> TestResult<BufReader<UnixStream>> {
        let stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut connection = BufReader::new(stream);
        let hello = json!({"jsonrpc": "2.0", "id": "h", "method": "hello", "params": {
            "generation": "1", "client": {"name": "t31", "version": "0"}, "credential": bearer,
        }});
        writeln!(connection.get_mut(), "{hello}")?;
        let mut line = String::new();
        connection.read_line(&mut line)?;
        let reply: Value = serde_json::from_str(&line)?;
        if reply.get("result").is_none() {
            return Err(reply.to_string().into());
        }
        Ok(connection)
    }
}
