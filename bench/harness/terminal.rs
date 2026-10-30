//! One echoing terminal: `cat` with terminal echo off, so every byte written
//! comes back exactly once.

use std::time::{Duration, Instant};

use ovis_terminal_probes::platform::{Grid, Pid, ProbeError, PtySession};
use portable_pty::CommandBuilder;

use super::error::HarnessError;

/// End-of-file in canonical mode; `cat` exits when it reads it on an empty line.
const EOF: &[u8] = b"\x04";

pub(super) struct EchoTerminal {
    session: PtySession,
    program: Pid,
    index: u16,
    rounds: u64,
}

impl EchoTerminal {
    pub(super) fn spawn(index: u16, bound: Duration) -> Result<Self, HarnessError> {
        let mut command = CommandBuilder::new("sh");
        command.args(["-c", r"stty -echo; printf 'HARNESS-READY\n'; exec cat"]);
        let mut session = PtySession::spawn(command, Grid::new(120, 40))?;
        session.read_until(b"HARNESS-READY\r\n", bound)?;
        let program = session.pid().ok_or(ProbeError::OutputEnded)?;
        Ok(Self {
            session,
            program,
            index,
            rounds: 0,
        })
    }

    pub(super) const fn program(&self) -> Pid {
        self.program
    }

    pub(super) const fn received(&self) -> u64 {
        self.session.received().get()
    }

    /// Writes `load` and a fresh marker line, and returns the time until the
    /// marker came back; the bytes that came back must be exactly those written.
    pub(super) fn round_trip(
        &mut self,
        load: &[u8],
        bound: Duration,
    ) -> Result<Duration, HarnessError> {
        self.rounds = self.rounds.saturating_add(1);
        let marker = format!("<{}:{}>", self.index, self.rounds);
        let mut expected = Vec::with_capacity(load.len().saturating_add(marker.len()));
        expected.extend_from_slice(load);
        expected.extend_from_slice(marker.as_bytes());
        let mut payload = expected.clone();
        payload.push(b'\n');
        let started = Instant::now();
        self.session.write(&payload)?;
        let echoed = self.session.read_until(marker.as_bytes(), bound)?;
        let elapsed = started.elapsed();
        // The terminal maps each written `\n` to `\r\n` on output (ONLCR).
        let without_returns: Vec<u8> = echoed.into_iter().filter(|byte| *byte != b'\r').collect();
        if !without_returns.ends_with(&expected) {
            return Err(HarnessError::EchoMismatch {
                terminal: self.index,
            });
        }
        Ok(elapsed)
    }

    /// Ends `cat` with end-of-file and joins the session's threads.
    pub(super) fn close(mut self, bound: Duration) -> Result<(), HarnessError> {
        self.session.write(EOF)?;
        self.session.finish(bound)?;
        Ok(())
    }
}

/// `lines` lines of 63 `x` bytes plus a newline.
pub(super) fn load_lines(lines: usize) -> Vec<u8> {
    let mut line = vec![b'x'; 63];
    line.push(b'\n');
    line.repeat(lines)
}
