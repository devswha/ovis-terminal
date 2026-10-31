//! The scheduler tuning harness of plan todo 56: parse budgets, the 16-PTY load profile
//! and the 50-terminal stress, each reported as one JSON document.
//!
//! - `parse`: per-turn parse time for each budget against one unbudgeted feed;
//! - `load <terminals> <viewers> <bytes-per-second> <seconds> [parse-budget [workers]]`:
//!   real PTY emitters, terminal actors, 16 request lanes and eight control-event clients
//!   per terminal; one viewer per terminal never reads; workers default to the host's
//!   parallelism clamped to 2..=8;
//! - `emit <bytes-per-second> <seconds>`: the paced emitter each load PTY runs.

mod clients;
mod corpus;
mod error;
mod fleet;
mod load;
mod parse;
mod pty;

use std::io::{self, Write as _};
use std::time::{Duration, Instant};

pub use error::TuningError;

/// Runs the mode `args` name and returns its JSON report; `emit` returns nothing.
///
/// # Errors
///
/// Returns `usage` for unknown arguments, or the failure of the run.
pub fn run(args: &[String]) -> Result<Vec<u8>, TuningError> {
    let count = |arg: &String| arg.parse::<usize>().map_err(|_| TuningError::Usage);
    match args {
        [mode] if mode == "parse" => Ok(serde_json::to_vec_pretty(&parse::run()?)?),
        [mode, terminals, viewers, rate, seconds, budget @ ..] if mode == "load" => {
            let nonzero = |arg: &String| count(arg)?.try_into().map_err(|_| TuningError::Usage);
            let (parse_budget, workers) = match budget {
                [] => (None, None),
                [bytes] => (Some(nonzero(bytes)?), None),
                [bytes, workers] => (Some(nonzero(bytes)?), Some(nonzero(workers)?)),
                _ => return Err(TuningError::Usage),
            };
            let load = load::Load {
                terminals: count(terminals)?,
                viewers: count(viewers)?,
                bytes_per_second: count(rate)?,
                seconds: seconds.parse().map_err(|_| TuningError::Usage)?,
                parse_budget,
                workers,
            };
            let exe = std::env::current_exe()?;
            Ok(serde_json::to_vec_pretty(&load::run(&exe, load)?)?)
        }
        [mode, rate, seconds] if mode == "emit" => {
            let seconds = seconds.parse().map_err(|_| TuningError::Usage)?;
            emit(count(rate)?, seconds)?;
            Ok(Vec::new())
        }
        _ => Err(TuningError::Usage),
    }
}

/// Writes `rate` bytes of terminal output per second, in 100 paced slices, for `seconds`.
fn emit(rate: usize, seconds: u64) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    let start = Instant::now();
    for second in 0..seconds {
        let seed = usize::try_from(second).unwrap_or_default();
        let bytes = corpus::second(rate, seed);
        let slice = bytes.len().div_ceil(100).max(1);
        for (tick, piece) in (0_u32..).zip(bytes.chunks(slice)) {
            stdout.write_all(piece)?;
            stdout.flush()?;
            let due = Duration::from_secs(second)
                .saturating_add(Duration::from_millis(10).saturating_mul(tick.saturating_add(1)));
            std::thread::sleep(due.saturating_sub(start.elapsed()));
        }
    }
    Ok(())
}
