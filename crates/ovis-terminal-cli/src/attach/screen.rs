//! The outer terminal while attached: raw mode when interacting, and everything drawn on
//! it tracked so the modes the program switched on are switched off again when the
//! guard drops, on every exit path, before the saved settings are put back.

use std::io;

use super::AttachError;
use super::modes::ModeTracker;
use super::tty::{self, RawMode};

/// Raw mode and drawn modes, both undone when dropped: modes first, then settings.
#[derive(Debug)]
pub(super) struct Screen {
    tracker: ModeTracker,
    stdout_tty: bool,
    /// Held only to restore the saved settings when dropped, after the modes reset.
    _raw: Option<RawMode>,
}

impl Screen {
    /// Takes over the outer terminal; raw mode only when `interact`.
    pub(super) fn open(interact: bool) -> Result<Self, AttachError> {
        let raw = if interact {
            Some(RawMode::enter()?)
        } else {
            None
        };
        Ok(Self {
            tracker: ModeTracker::default(),
            stdout_tty: tty::stdout_is_tty(),
            _raw: raw,
        })
    }

    /// Draws terminal bytes on stdout, unchanged; a terminal that hung up (EIO) is
    /// [`AttachError::TerminalGone`], which detaches instead of failing. The bytes go
    /// straight to the descriptor on the blocking pool, never through the process's
    /// buffered stdout, so a failed draw leaves nothing queued for the exit report's
    /// flush to fail on.
    pub(super) async fn draw(&mut self, bytes: &[u8]) -> Result<(), AttachError> {
        self.tracker.track(bytes);
        let chunk = bytes.to_vec();
        let written = tokio::task::spawn_blocking(move || tty::write_unbuffered(&chunk))
            .await
            .unwrap_or_else(|join| Err(io::Error::other(join)));
        written.map_err(|source| {
            if source.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) {
                AttachError::TerminalGone
            } else {
                AttachError::Io {
                    operation: "write the screen to stdout",
                    source,
                }
            }
        })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let reset = self.tracker.reset();
        if !self.stdout_tty || reset.is_empty() {
            return;
        }
        if let Err(error) = tty::write_unbuffered(&reset) {
            tty::warn("reset terminal modes", &error.to_string());
        }
    }
}
