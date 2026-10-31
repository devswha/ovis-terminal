//! `ovis-terminal` binary entry point.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::ExitCode;

use ovis_terminal_cli::{Context, ExitStatus, run};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let context = Context::from_process();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return match writeln!(
                io::stderr().lock(),
                "ovis-terminal: no async runtime: {error}"
            ) {
                Ok(()) | Err(_) => ExitStatus::Failure.into(),
            };
        }
    };
    let report = runtime.block_on(run(args, &context));
    // A blocking stdin read from `connect` must not hold the process open after the reply.
    runtime.shutdown_background();
    report.emit()
}
