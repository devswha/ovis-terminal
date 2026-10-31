//! Opening a connection: through the binary's `connect` bridge or a direct socket.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};
use tokio::process::{Child, ChildStderr};

use crate::error::RunnerError;
use crate::evidence::{CapturedText, ProcessReport};
use crate::ids::Decimal;
use crate::scenario::model::Via;

/// Bound on a bridge process exiting after its stdin closes.
const BRIDGE_EXIT_BOUND: Duration = Duration::from_secs(10);

pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;
pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;

/// Where the fixture listens and which binary bridges to it.
#[derive(Debug, Clone)]
pub(crate) struct Endpoint {
    pub(crate) binary: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) socket: PathBuf,
}

impl Endpoint {
    /// Applies the isolated environment: nothing from the runner's own session leaks in.
    pub(crate) fn isolate(&self, command: &mut tokio::process::Command) {
        command
            .env_clear()
            .env("HOME", &self.root)
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("OVIS_TERMINAL_SOCKET", &self.socket);
    }
}

/// The `connect` process carrying one connection, owned until it is reaped.
#[derive(Debug)]
pub(crate) struct Bridge {
    child: Child,
    stderr: Option<ChildStderr>,
    argv: Vec<String>,
}

pub(crate) struct Opened {
    pub(crate) reader: Reader,
    pub(crate) writer: Writer,
    pub(crate) bridge: Option<Bridge>,
}

pub(crate) async fn open(via: Via, endpoint: &Endpoint) -> Result<Opened, RunnerError> {
    match via {
        Via::Bridge => bridge(endpoint),
        Via::Socket => socket(&endpoint.socket).await,
    }
}

fn bridge(endpoint: &Endpoint) -> Result<Opened, RunnerError> {
    let mut command = tokio::process::Command::new(&endpoint.binary);
    command
        .arg("connect")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    endpoint.isolate(&mut command);
    let mut child = command.spawn().map_err(|source| RunnerError::Spawn {
        program: endpoint.binary.clone(),
        source,
    })?;
    let missing = || RunnerError::Io {
        operation: "take the bridge's piped streams",
        source: std::io::Error::other("stream was not piped"),
    };
    let writer: Writer = Box::new(child.stdin.take().ok_or_else(missing)?);
    let reader: Reader = Box::new(child.stdout.take().ok_or_else(missing)?);
    let stderr = child.stderr.take();
    let argv = vec![endpoint.binary.display().to_string(), "connect".to_owned()];
    Ok(Opened {
        reader,
        writer,
        bridge: Some(Bridge {
            child,
            stderr,
            argv,
        }),
    })
}

#[cfg(unix)]
async fn socket(path: &Path) -> Result<Opened, RunnerError> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|source| RunnerError::Io {
            operation: "connect to the fixture socket",
            source,
        })?;
    let (reader, writer) = stream.into_split();
    Ok(Opened {
        reader: Box::new(reader),
        writer: Box::new(writer),
        bridge: None,
    })
}

#[cfg(not(unix))]
async fn socket(_: &Path) -> Result<Opened, RunnerError> {
    Err(RunnerError::UnsupportedPlatform)
}

impl Bridge {
    /// Waits, bounded, for the bridge to exit after its stdin closed; kills only this child.
    pub(crate) async fn reap(mut self) -> ProcessReport {
        let pid = self.child.id().map(|pid| Decimal(u64::from(pid)));
        let mut stderr = Vec::new();
        let exit = match tokio::time::timeout(BRIDGE_EXIT_BOUND, self.child.wait()).await {
            Ok(Ok(status)) => status.code(),
            Ok(Err(error)) => {
                stderr.extend_from_slice(format!("<wait failed: {error}>").as_bytes());
                None
            }
            Err(_) => {
                let note = match self.child.start_kill() {
                    Ok(()) => "<killed after the exit bound>".to_owned(),
                    Err(error) => format!("<kill after the exit bound failed: {error}>"),
                };
                stderr.extend_from_slice(note.as_bytes());
                None
            }
        };
        if let Some(mut pipe) = self.stderr.take() {
            let read = tokio::time::timeout(BRIDGE_EXIT_BOUND, pipe.read_to_end(&mut stderr)).await;
            if !matches!(read, Ok(Ok(_))) {
                stderr.extend_from_slice(b"<stderr unreadable>");
            }
        }
        ProcessReport {
            role: "bridge",
            argv: self.argv,
            pid,
            exit_code: exit,
            stdout: CapturedText::of(b""),
            stderr: CapturedText::of(&stderr),
        }
    }
}
