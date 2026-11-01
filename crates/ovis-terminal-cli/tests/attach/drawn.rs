//! What an observer drew on its stdout, read on a thread as it arrives.

use std::io::Read as _;
use std::process::ChildStdout;
use std::sync::mpsc;
use std::time::Instant;

use super::WAIT;

/// The observer's drawn output so far.
#[derive(Debug)]
pub struct Drawn {
    output: mpsc::Receiver<Vec<u8>>,
    seen: Vec<u8>,
}

impl Drawn {
    /// Follows `stdout` until it closes.
    pub fn follow(mut stdout: ChildStdout) -> Self {
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            while let Ok(read) = stdout.read(&mut chunk) {
                let Some(bytes) = chunk.get(..read).filter(|bytes| !bytes.is_empty()) else {
                    break;
                };
                if sender.send(bytes.to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            output,
            seen: Vec::new(),
        }
    }

    /// Waits, bounded by [`WAIT`], until `needle` was drawn.
    pub fn shows(&mut self, needle: &str) -> bool {
        let Some(deadline) = Instant::now().checked_add(WAIT) else {
            return false;
        };
        loop {
            if self
                .seen
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(chunk) => self.seen.extend(chunk),
                Err(_) => return false,
            }
        }
    }
}
