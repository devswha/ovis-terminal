//! The local release server: static GETs below one directory on a loopback port.
//!
//! Paths below `/stall/` serve the same files, except that an archive (`.tar.gz`) sends
//! its full `Content-Length` and only half its bytes, reports that on a channel and holds
//! the connection open until the probe releases it: a download cancelled mid-transfer.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::error::InstallProbeError;

const STALL_PREFIX: &str = "stall/";
const REQUEST_LINES: usize = 64;

/// A running release server; dropping it stops the accept loop and joins it.
pub(super) struct Server {
    addr: SocketAddr,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    stalled: Receiver<()>,
    release: Sender<()>,
}

struct Shared {
    root: PathBuf,
    stalled: Sender<()>,
    release: Mutex<Receiver<()>>,
}

impl Server {
    /// Serves `root` on `127.0.0.1` at a free port.
    pub(super) fn start(root: &Path) -> Result<Self, InstallProbeError> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(InstallProbeError::io("listen on", root))?;
        let addr = listener
            .local_addr()
            .map_err(InstallProbeError::io("read the address of", root))?;
        let (stalled_tx, stalled) = channel();
        let (release, release_rx) = channel();
        let shared = Arc::new(Shared {
            root: root.to_path_buf(),
            stalled: stalled_tx,
            release: Mutex::new(release_rx),
        });
        let stopping = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopping);
        let thread = std::thread::spawn(move || accept(&listener, &flag, &shared));
        Ok(Self {
            addr,
            stopping,
            thread: Some(thread),
            stalled,
            release,
        })
    }

    /// `http://127.0.0.1:<port>/<tree>`.
    pub(super) fn url(&self, tree: &str) -> String {
        format!("http://{}/{tree}", self.addr)
    }

    /// Waits until a stalled download has sent half its archive.
    pub(super) fn await_stall(&self, bound: std::time::Duration) -> Result<(), InstallProbeError> {
        self.stalled
            .recv_timeout(bound)
            .map_err(|_| InstallProbeError::HangGuard {
                what: "the stalled download",
                bound_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
            })
    }

    /// Lets a stalled connection close.
    pub(super) fn release_stall(&self) {
        // The connection may already be gone with its client; nothing waits then.
        if self.release.send(()).is_err() {
            tracing::debug!("no stalled connection to release");
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.release_stall();
        // Wakes the blocking accept so the loop sees the flag.
        if TcpStream::connect(self.addr).is_err() {
            tracing::debug!("the release server had already stopped accepting");
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::warn!("the release server thread panicked");
        }
    }
}

fn accept(listener: &TcpListener, stopping: &AtomicBool, shared: &Arc<Shared>) {
    for stream in listener.incoming() {
        if stopping.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let shared = Arc::clone(shared);
        std::thread::spawn(move || {
            if let Err(error) = respond(stream, &shared) {
                tracing::debug!(%error, "release server connection ended");
            }
        });
    }
}

fn request_path(stream: &TcpStream) -> std::io::Result<Option<String>> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    for _ in 0..REQUEST_LINES {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let mut parts = first.split_whitespace();
    Ok(match (parts.next(), parts.next()) {
        (Some("GET"), Some(path)) => Some(path.trim_start_matches('/').to_owned()),
        _ => None,
    })
}

fn local_file(root: &Path, path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    let safe = relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)));
    let file = root.join(relative);
    (safe && file.is_file()).then_some(file)
}

fn respond(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some(path) = request_path(&stream)? else {
        return stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    };
    let stall = path.starts_with(STALL_PREFIX);
    let served = path.strip_prefix(STALL_PREFIX).unwrap_or(&path);
    let Some(file) = local_file(&shared.root, served) else {
        return stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let body = std::fs::read(file)?;
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    if stall && served.ends_with(".tar.gz") {
        stream.write_all(body.get(..body.len() / 2).unwrap_or_default())?;
        stream.flush()?;
        // The probe may have gone; the stalled connection then just closes.
        if shared.stalled.send(()).is_err() {
            tracing::debug!("nobody awaits the stall");
        }
        let released = shared
            .release
            .lock()
            .map_err(|_| std::io::Error::other("stall lock"))?
            .recv();
        if released.is_err() {
            tracing::debug!("the release server stopped while stalled");
        }
        return Ok(());
    }
    stream.write_all(&body)
}
