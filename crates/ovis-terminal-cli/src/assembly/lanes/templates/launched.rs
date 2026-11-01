//! The launched terminal T35 answers, read from the runtime's `terminal.run` result.

use ovis_terminal_protocol::domain::template_launch::LaunchedTerminal;
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
use ovis_terminal_server::dispatch::DispatchError;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn field<T: DeserializeOwned>(result: &Value, pointer: &str) -> Result<T, DispatchError> {
    serde_json::from_value(result.pointer(pointer).cloned().unwrap_or(Value::Null))
        .map_err(Into::into)
}

/// The launched terminal of `result`; the request's metadata comes from `params`.
pub(super) fn from_run(
    result: &Value,
    params: &TerminalRunParams,
) -> Result<LaunchedTerminal, DispatchError> {
    let identity = (
        field(result, "/terminal/terminal_id")?,
        field(result, "/terminal/address")?,
        field(result, "/terminal/machine_id")?,
    );
    let process = (
        field(result, "/terminal/process")?,
        field(result, "/created_seq")?,
    );
    Ok(LaunchedTerminal::new(identity, process, params))
}
