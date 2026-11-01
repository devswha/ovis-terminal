//! `ideal-audit`: see `ovis_terminal_probes::quality`.

use std::process::ExitCode;

use ovis_terminal_probes::quality::{Tool, main as run};

fn main() -> ExitCode {
    run(Tool::IdealAudit)
}
