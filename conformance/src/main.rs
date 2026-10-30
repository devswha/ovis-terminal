//! Real-socket conformance runner for the `ovis-terminal` binary.
//!
//! The runner contract (isolated session root, endpoint-ready signal, typed
//! evidence files) is implemented by plan todo 21. Until then this binary
//! registers no scenarios and exits with a failure status, so no scaffold run
//! can be mistaken for a passing conformance result.

use std::io::{self, Write as _};
use std::process::ExitCode;

/// Exit status reported when no scenario can be executed.
const NO_SCENARIOS_EXIT_CODE: u8 = 2;

fn main() -> ExitCode {
    let reported = writeln!(
        io::stderr().lock(),
        "conformance: no scenarios are registered in this build (protocol crate {})",
        ovis_terminal_protocol::CRATE_NAME
    );
    match reported {
        Ok(()) => ExitCode::from(NO_SCENARIOS_EXIT_CODE),
        Err(_) => ExitCode::FAILURE,
    }
}
