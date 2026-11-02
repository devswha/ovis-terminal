//! `install-probe`: see `ovis_terminal_probes::install`.

use std::process::ExitCode;

fn main() -> ExitCode {
    ovis_terminal_probes::install::main()
}
