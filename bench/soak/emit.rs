//! The program every terminal runs: numbered text lines paced to a byte rate, until its
//! terminal goes away.

use std::io::{self, Write as _};
use std::time::{Duration, Instant};

const TICKS_PER_SECOND: usize = 20;
const TICK: Duration = Duration::from_millis(50);

/// Writes about `bytes_per_second` bytes a second to stdout until a write fails.
pub(super) fn run(bytes_per_second: usize) -> io::Result<()> {
    let per_tick = bytes_per_second
        .checked_div(TICKS_PER_SECOND)
        .unwrap_or(0)
        .max(1);
    let mut out = io::stdout().lock();
    let mut line = 0_u64;
    let mut chunk = Vec::with_capacity(per_tick.saturating_add(64));
    let mut next = Instant::now();
    loop {
        chunk.clear();
        while chunk.len() < per_tick {
            write!(chunk, "{line:016} soak output line\r\n")?;
            line = line.wrapping_add(1);
        }
        out.write_all(&chunk)?;
        out.flush()?;
        next = next.checked_add(TICK).unwrap_or_else(Instant::now);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}
