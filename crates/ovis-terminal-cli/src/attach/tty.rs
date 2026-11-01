//! The outer terminal: whether stdin and stdout are terminals, their grid, and raw mode
//! held by a guard that puts the saved settings back on every exit path.

use std::io;

use rustix::termios::{self, OptionalActions, Termios};
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc;

use super::AttachError;

fn tty_error(operation: &'static str) -> impl FnOnce(rustix::io::Errno) -> AttachError {
    move |errno| AttachError::Io {
        operation,
        source: io::Error::from(errno),
    }
}

/// Whether both stdin and stdout are terminals.
pub(super) fn interactive() -> bool {
    termios::isatty(rustix::stdio::stdin()) && termios::isatty(rustix::stdio::stdout())
}

/// The outer terminal's grid as columns and rows.
pub(super) fn grid() -> Result<(u16, u16), AttachError> {
    let size = termios::tcgetwinsize(rustix::stdio::stdout())
        .map_err(tty_error("read the terminal size"))?;
    Ok((size.ws_col, size.ws_row))
}

/// Raw mode on stdin for the guard's lifetime; dropping it restores the saved settings.
#[derive(Debug)]
pub(super) struct RawMode {
    saved: Termios,
}

impl RawMode {
    /// Saves the current settings, then switches stdin to raw mode.
    pub(super) fn enter() -> Result<Self, AttachError> {
        let stdin = rustix::stdio::stdin();
        let saved = termios::tcgetattr(stdin).map_err(tty_error("read terminal settings"))?;
        let mut raw = saved.clone();
        raw.make_raw();
        termios::tcsetattr(stdin, OptionalActions::Drain, &raw)
            .map_err(tty_error("enter raw mode"))?;
        Ok(Self { saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let restored =
            termios::tcsetattr(rustix::stdio::stdin(), OptionalActions::Now, &self.saved);
        if let Err(errno) = restored {
            let line = serde_json::json!({"warning": {"stage": "restore terminal settings", "detail": errno.to_string()}});
            match io::Write::write_all(&mut io::stderr().lock(), format!("{line}\n").as_bytes()) {
                Ok(()) | Err(_) => {}
            }
        }
    }
}

/// Typed bytes from stdin, read off the async runtime's blocking pool.
pub(super) fn keyboard() -> mpsc::Receiver<Vec<u8>> {
    let (sender, typed) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buffer = vec![0_u8; 4096];
        loop {
            let read = match stdin.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let chunk = buffer.get(..read).map(<[u8]>::to_vec).unwrap_or_default();
            if sender.send(chunk).await.is_err() {
                break;
            }
        }
    });
    typed
}

pub(super) async fn typed_or_never(
    keyboard: Option<&mut mpsc::Receiver<Vec<u8>>>,
) -> Option<Vec<u8>> {
    match keyboard {
        Some(keyboard) => keyboard.recv().await,
        None => std::future::pending().await,
    }
}
