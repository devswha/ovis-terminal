//! A running transport: protocol bytes on stdin/stdout, diagnostics drained from stderr.

use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

use super::error::Evidence;
use super::{TransportError, TransportSpec};

/// Largest reply line accepted, the protocol's control-record bound.
const MAX_LINE_BYTES: usize = ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;
/// Stderr bytes kept as diagnostics; the rest is drained and dropped.
const DIAGNOSTIC_BYTES: usize = 8 * 1024;
/// How long a closed transport gets to report its exit status.
const EXIT_BOUND: Duration = Duration::from_secs(2);

/// One spawned transport. Dropping it kills only this child, by its own handle.
#[derive(Debug)]
pub struct Link {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    partial: Vec<u8>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
    drain: JoinHandle<()>,
}

impl Link {
    /// Starts the transport with piped standard streams and no inherited terminal.
    ///
    /// # Errors
    ///
    /// [`TransportError::Spawn`] when the program cannot start.
    pub fn spawn(spec: &TransportSpec) -> Result<Self, TransportError> {
        let argv = spec.argv();
        let spawn_error = |source| TransportError::Spawn {
            program: argv.first().cloned().unwrap_or_default(),
            source,
        };
        let mut command = spec
            .command()
            .ok_or_else(|| spawn_error(std::io::ErrorKind::InvalidInput.into()))?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(spawn_error)?;
        let missing = || spawn_error(std::io::ErrorKind::BrokenPipe.into());
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = child.stdout.take().ok_or_else(missing)?;
        let mut stderr = child.stderr.take().ok_or_else(missing)?;
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&diagnostics);
        let drain = tokio::spawn(async move {
            let mut chunk = [0_u8; 4096];
            while let Ok(read @ 1..) = stderr.read(&mut chunk).await {
                let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
                let room = DIAGNOSTIC_BYTES.saturating_sub(kept.len());
                kept.extend(chunk.iter().take(read.min(room)));
            }
        });
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            partial: Vec::new(),
            diagnostics,
            drain,
        })
    }

    /// Writes `bytes` to the transport's stdin unchanged.
    ///
    /// # Errors
    ///
    /// [`TransportError::Io`] when the transport no longer reads.
    pub async fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.stdin.write_all(bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Reads one newline-terminated record, without the newline. Cancel safe: bytes of an
    /// unfinished record stay in the link and are reported when the handshake times out.
    ///
    /// # Errors
    ///
    /// [`TransportError::Closed`] at end of output and [`TransportError::NotProtocol`] for a
    /// record over the protocol bound.
    pub async fn receive(&mut self) -> Result<Vec<u8>, TransportError> {
        loop {
            let available = self.stdout.fill_buf().await?;
            if available.is_empty() {
                return Err(self.closed().await);
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let taken = newline.map_or(available.len(), |at| at.saturating_add(1));
            self.partial.extend(available.iter().take(taken));
            self.stdout.consume(taken);
            if self.partial.len() > MAX_LINE_BYTES {
                return Err(TransportError::NotProtocol {
                    first_bytes: Evidence::of(self.preview()),
                });
            }
            if newline.is_some() {
                let mut line = std::mem::take(&mut self.partial);
                line.pop();
                return Ok(line);
            }
        }
    }

    /// Bytes of a record that has not ended yet, bounded.
    pub(super) fn received(&self) -> Evidence {
        Evidence::of(self.preview())
    }

    /// What the transport has written to stderr so far, bounded.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<u8> {
        self.diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn preview(&self) -> &[u8] {
        self.partial.get(..256).unwrap_or(&self.partial)
    }

    async fn closed(&mut self) -> TransportError {
        let status = tokio::time::timeout(EXIT_BOUND, self.child.wait()).await;
        let status = status
            .ok()
            .and_then(Result::ok)
            .and_then(|status| status.code());
        // The drain ends at stderr EOF, which follows the exit; wait for it, bounded.
        let drained = tokio::time::timeout(EXIT_BOUND, &mut self.drain).await;
        drop(drained);
        TransportError::Closed {
            status,
            diagnostics: Evidence::of(&self.diagnostics()),
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.drain.abort();
    }
}
