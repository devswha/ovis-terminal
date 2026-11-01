//! The outer terminal while attached: raw mode when interacting, and everything drawn on
//! it tracked so the modes the program switched on are switched off again when the
//! guard drops, on every exit path, before the saved settings are put back.

use std::io::Write as _;

use tokio::io::AsyncWriteExt as _;

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

    /// Draws terminal bytes on stdout, unchanged.
    pub(super) async fn draw(&mut self, bytes: &[u8]) -> Result<(), AttachError> {
        self.tracker.track(bytes);
        let mut stdout = tokio::io::stdout();
        let written = stdout.write_all(bytes).await.and(stdout.flush().await);
        written.map_err(|source| AttachError::Io {
            operation: "write the screen to stdout",
            source,
        })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let reset = self.tracker.reset();
        if !self.stdout_tty || reset.is_empty() {
            return;
        }
        let mut stdout = std::io::stdout().lock();
        if let Err(error) = stdout.write_all(&reset).and_then(|()| stdout.flush()) {
            tty::warn("reset terminal modes", &error.to_string());
        }
    }
}
