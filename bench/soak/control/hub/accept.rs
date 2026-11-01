//! The control socket: an accept thread and one reader thread per viewer, all stoppable
//! and joined. A failed accept is counted and survived; it never ends the listener.

use std::io::{BufRead as _, BufReader};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use super::super::super::error::SoakError;
use super::super::super::wire::{Hello, Request};
use super::super::Inbound;

/// Pause after a failed accept, so a persistent failure (no descriptors) cannot spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(10);

/// What a viewer connection reports to the control loop.
#[derive(Debug)]
pub(in super::super::super) enum ViewerMsg {
    Joined {
        id: u64,
        hello: Hello,
        stream: UnixStream,
    },
    Asked {
        id: u64,
        request: Request,
    },
    Left {
        id: u64,
    },
}

/// State the accept thread and the control loop share; no lock is held across I/O.
#[derive(Debug, Default)]
struct Shared {
    stop: AtomicBool,
    failures: AtomicU64,
    readers: Mutex<Vec<(UnixStream, JoinHandle<()>)>>,
}

#[derive(Debug)]
pub(super) struct Accept {
    socket: PathBuf,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

fn read(id: u64, (stream, inbound): (UnixStream, &SyncSender<Inbound>)) {
    let left = || {
        inbound
            .send(Inbound::Viewer(ViewerMsg::Left { id }))
            .is_ok()
    };
    let Ok(reading) = stream.try_clone() else {
        left();
        return;
    };
    let mut lines = BufReader::new(reading).lines();
    let hello = lines
        .next()
        .and_then(Result::ok)
        .and_then(|line| serde_json::from_str::<Hello>(&line).ok());
    let Some(hello) = hello else {
        left();
        return;
    };
    let joined = ViewerMsg::Joined { id, hello, stream };
    if inbound.send(Inbound::Viewer(joined)).is_err() {
        return;
    }
    for line in lines {
        let request = line
            .ok()
            .and_then(|text| serde_json::from_str::<Request>(&text).ok());
        let Some(request) = request else { break };
        if inbound
            .send(Inbound::Viewer(ViewerMsg::Asked { id, request }))
            .is_err()
        {
            return;
        }
    }
    left();
}

fn serve(listener: &UnixListener, (inbound, accept): (&SyncSender<Inbound>, &Shared)) {
    let mut next = 0_u64;
    for incoming in listener.incoming() {
        if accept.stop.load(Ordering::Acquire) {
            return;
        }
        let Ok(stream) = incoming else {
            accept.failures.fetch_add(1, Ordering::AcqRel);
            std::thread::sleep(ACCEPT_BACKOFF);
            continue;
        };
        next = next.saturating_add(1);
        let (Ok(kept), Ok(served)) = (stream.try_clone(), stream.try_clone()) else {
            accept.failures.fetch_add(1, Ordering::AcqRel);
            continue;
        };
        let sender = inbound.clone();
        let id = next;
        let reader = std::thread::Builder::new()
            .name("soak-viewer-reader".to_owned())
            .spawn(move || read(id, (served, &sender)));
        match reader {
            Ok(reader) => accept
                .readers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((kept, reader)),
            Err(_) => {
                accept.failures.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

impl Accept {
    pub(super) fn bind(socket: &Path, inbound: SyncSender<Inbound>) -> Result<Self, SoakError> {
        // A crashed predecessor leaves its socket file; it is cleared only when nobody
        // answers on it, so a live boot is never displaced.
        if socket.exists() {
            match UnixStream::connect(socket) {
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(socket)
                        .map_err(SoakError::io("clear a dead control socket"))?;
                }
                Ok(_) => {
                    return Err(SoakError::Protocol {
                        expected: "no live control boot on the control socket",
                    });
                }
                Err(source) => return Err(SoakError::io("probe the control socket")(source)),
            }
        }
        let listener =
            UnixListener::bind(socket).map_err(SoakError::io("bind the control socket"))?;
        let shared = Arc::new(Shared::default());
        let serving = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("soak-accept".to_owned())
            .spawn(move || serve(&listener, (&inbound, &serving)))
            .map_err(SoakError::io("start the accept thread"))?;
        Ok(Self {
            socket: socket.to_path_buf(),
            shared,
            thread: Some(thread),
        })
    }

    /// Accepts and reader starts that failed.
    pub(super) fn failures(&self) -> u64 {
        self.shared.failures.load(Ordering::Acquire)
    }

    /// Stops accepting, ends every connection and joins every thread.
    pub(super) fn close(mut self) -> Result<(), SoakError> {
        self.shared.stop.store(true, Ordering::Release);
        // A connection of our own wakes the blocked accept; refusal means it already ended.
        let woke = UnixStream::connect(&self.socket);
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| SoakError::Protocol {
                expected: "an accept thread that ends",
            })?;
        }
        drop(woke);
        let readers = std::mem::take(
            &mut *self
                .shared
                .readers
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for (stream, reader) in readers {
            match stream.shutdown(Shutdown::Both) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotConnected => {}
                Err(source) => return Err(SoakError::io("end a viewer connection")(source)),
            }
            reader.join().map_err(|_| SoakError::Protocol {
                expected: "a viewer reader that ends",
            })?;
        }
        std::fs::remove_file(&self.socket).map_err(SoakError::io("remove the control socket"))
    }
}
