//! The paced program each PTY runs: colored build-log output at a fixed byte rate until
//! its terminal goes away. It consumes its input, so admitted bytes reach a reader and a
//! full PTY can never hold the stdin forwarder at teardown.

use std::io::{self, Read as _, Write as _};
use std::time::{Duration, Instant};

/// `len` bytes of colored build-log lines with cursor moves and erases, from `seed`.
pub(super) fn corpus(len: usize, seed: usize) -> Vec<u8> {
    let mut unit = Vec::new();
    for line in 0..64_usize {
        let colour = 31_usize.saturating_add(line % 7);
        let text = format!(
            "\x1b[{colour};1m[{line:03}]\x1b[0m compiling crate-{line} v0.1.0 (/src/{line})\x1b[K\r\n"
        );
        unit.extend_from_slice(text.as_bytes());
    }
    unit.iter()
        .copied()
        .cycle()
        .skip(seed.checked_rem(unit.len()).unwrap_or(0))
        .take(len)
        .collect()
}

/// Writes `rate` bytes per second in 100 paced slices until stdout fails, while a thread
/// reads and discards stdin.
pub(super) fn run(rate: usize) -> io::Result<()> {
    std::thread::Builder::new()
        .name("emit-stdin".to_owned())
        .spawn(|| {
            let mut stdin = io::stdin().lock();
            let mut sink = [0_u8; 4096];
            while matches!(stdin.read(&mut sink), Ok(read) if read > 0) {}
        })?;
    let mut stdout = io::stdout().lock();
    let start = Instant::now();
    let slice = rate.div_ceil(100).max(1);
    for tick in 1_u32.. {
        let bytes = corpus(slice, usize::try_from(tick).unwrap_or_default());
        if stdout
            .write_all(&bytes)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return Ok(());
        }
        let due = Duration::from_millis(10).saturating_mul(tick);
        std::thread::sleep(due.saturating_sub(start.elapsed()));
    }
    Ok(())
}
