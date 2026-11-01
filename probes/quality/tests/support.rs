//! Helpers shared by the quality gate tests.

use ovis_terminal_probes::quality::{GateRun, Tool, execute};
use serde_json::Value;

/// Runs `tool` in-process with `args`.
pub fn run(tool: Tool, args: &[&str]) -> GateRun {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    execute(tool, &args)
}

/// The value at `pointer`, or null.
pub fn field<'a>(report: &'a Value, pointer: &str) -> &'a Value {
    report.pointer(pointer).unwrap_or(&Value::Null)
}
