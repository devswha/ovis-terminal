//! `soak` runs the continuous observation harness of plan todo 70 against the holders
//! of the binary under test; see `bench/soak/soak.rs`. Unix only: Windows stays at the
//! CI smoke tier.

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    ovis_terminal_bench::soak::main_with(std::env::args_os().skip(1).collect())
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    use std::io::Write as _;

    let line = b"usage: soak runs on Unix hosts only\n";
    if std::io::stderr().write_all(line).is_err() {
        return ExitCode::from(3);
    }
    ExitCode::from(2)
}
