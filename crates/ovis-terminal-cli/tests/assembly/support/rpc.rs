//! One JSON-RPC request on a raw connection, answered by id.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;

use serde_json::{Value, json};

use super::{TestResult, op_id};

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
