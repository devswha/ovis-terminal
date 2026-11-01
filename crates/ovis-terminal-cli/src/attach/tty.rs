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

/// Whether stdout is a terminal, which is when drawn modes are reset on exit.
pub(super) fn stdout_is_tty() -> bool {
    termios::isatty(rustix::stdio::stdout())
}

/// Reports a restore step that failed on stderr as one JSON warning line; the exit
/// record still follows.
pub(super) fn warn(stage: &str, detail: &str) {
    let line = serde_json::json!({"warning": {"stage": stage, "detail": detail}});
    match io::Write::write_all(&mut io::stderr().lock(), format!("{line}\n").as_bytes()) {
        Ok(()) | Err(_) => {}
    }
}

/// Writes `bytes` straight to the stdout descriptor, bypassing the process's buffered
/// stdout, so a failure on a hung-up terminal leaves nothing queued for the final report's
/// flush to fail on again.
pub(super) fn write_unbuffered(mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = match rustix::io::write(rustix::stdio::stdout(), bytes) {
            Err(rustix::io::Errno::INTR) => continue,
            written => written.map_err(io::Error::from)?,
        };
        if written == 0 {
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
        bytes = bytes.get(written..).unwrap_or_default();
    }
    Ok(())
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
            warn("restore terminal settings", &errno.to_string());
        }
    }
}

/// What the keyboard produced: typed bytes, or how reading it ended.
#[derive(Debug)]
pub(super) enum Typed {
    /// Bytes typed on the outer terminal.
    Bytes(Vec<u8>),
    /// The outer terminal hung up. Stdin is a raw terminal (VMIN 1), so a read only
    /// returns no bytes, or fails with EIO, once the terminal is gone.
    HungUp,
    /// Reading the keyboard failed for another reason.
    Failed(io::Error),
}

/// Typed bytes from stdin, read off the async runtime's blocking pool; the last item is
/// how reading ended.
pub(super) fn keyboard() -> mpsc::Receiver<Typed> {
    let (sender, typed) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buffer = vec![0_u8; 4096];
        loop {
            let item = match stdin.read(&mut buffer).await {
                Ok(0) => Typed::HungUp,
                Err(error)
                    if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
                {
                    Typed::HungUp
                }
                Err(error) => Typed::Failed(error),
                Ok(read) => {
                    Typed::Bytes(buffer.get(..read).map(<[u8]>::to_vec).unwrap_or_default())
                }
            };
            let ended = !matches!(item, Typed::Bytes(_));
            if sender.send(item).await.is_err() || ended {
                break;
            }
        }
    });
    typed
}

/// The keyboard's next item; never resolves when observing or once reading has ended.
pub(super) async fn typed_or_never(keyboard: Option<&mut mpsc::Receiver<Typed>>) -> Typed {
    match keyboard {
        Some(keyboard) => match keyboard.recv().await {
            Some(typed) => typed,
            None => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}
