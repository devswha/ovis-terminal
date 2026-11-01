//! `terminal50` benchmarks terminal actors on real PTYs against the published latency
//! budgets of plan todo 69; see `bench/terminal50/terminal50.rs`. Unix only: Windows
//! stays at the CI smoke tier.

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    ovis_terminal_bench::terminal50::main_with(std::env::args_os().skip(1).collect())
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    use std::io::Write as _;

    let line = b"usage: terminal50 runs on Unix hosts only\n";
    if std::io::stderr().write_all(line).is_err() {
        return ExitCode::from(3);
    }
    ExitCode::from(2)
}
