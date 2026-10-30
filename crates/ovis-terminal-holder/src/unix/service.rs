//! The holder socket service: one adopting core connection at a time.
//!
//! Connections are served one after another on the accept thread, so the
//! service runs at most two threads (accept plus one link writer), and every
//! thread is joined: stopping wakes the blocked `accept` by connecting to the
//! socket instead of waiting for a later accept error.

mod connection;

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{Builder, JoinHandle};

use super::error::HolderError;
use super::holder::Holder;
use super::service_error::{ConnectionError, ServeError};

struct ServiceContext {
    holder: Holder,
    socket: PathBuf,
    // SeqCst throughout: the flag is read once per accepted connection.
    stopping: AtomicBool,
    current: Mutex<Option<UnixStream>>,
    last_failure: Mutex<Option<ConnectionError>>,
}

/// A running holder socket service.
#[derive(Debug)]
#[must_use = "stop or wait for the service so its threads are joined"]
pub struct HolderService {
    context: Arc<ServiceContext>,
    accept: JoinHandle<Result<(), HolderError>>,
}

impl std::fmt::Debug for ServiceContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceContext")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

impl HolderService {
    /// Serves `holder` on a listener bound to a filesystem path.
    ///
    /// # Errors
    ///
    /// Returns [`ServeError::UnnamedSocket`] for an unnamed listener and
    /// [`ServeError::Io`] when the accept thread cannot start.
    pub fn start(listener: UnixListener, holder: Holder) -> Result<Self, ServeError> {
        let address = listener
            .local_addr()
            .map_err(|source| ServeError::Io { source })?;
        let socket = address
            .as_pathname()
            .ok_or(ServeError::UnnamedSocket)?
            .to_path_buf();
        let context = Arc::new(ServiceContext {
            holder,
            socket,
            stopping: AtomicBool::new(false),
            current: Mutex::new(None),
            last_failure: Mutex::new(None),
        });
        let accept_context = Arc::clone(&context);
        let accept = Builder::new()
            .name("ovis-holder-accept".to_owned())
            .spawn(move || accept_loop(&accept_context, &listener))
            .map_err(|source| ServeError::Io { source })?;
        Ok(Self { context, accept })
    }

    /// The holder this service serves.
    #[must_use]
    pub fn holder(&self) -> &Holder {
        &self.context.holder
    }

    /// Kind of the most recent connection failure, if any connection failed.
    #[must_use]
    pub fn last_failure_kind(&self) -> Option<&'static str> {
        self.context
            .last_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(ConnectionError::kind)
    }

    /// Stops serving, shuts the holder down and joins every thread.
    ///
    /// # Errors
    ///
    /// Returns [`ServeError::Io`] when the accept thread cannot be woken (it is
    /// then not joined), or the holder shutdown or join failure.
    pub fn stop(self) -> Result<(), ServeError> {
        self.context.stopping.store(true, Ordering::SeqCst);
        if let Some(stream) = self
            .context
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            // The connection may already be closing; its reader ends either way.
            stream
                .shutdown(std::net::Shutdown::Both)
                .unwrap_or_default();
        }
        UnixStream::connect(&self.context.socket).map_err(|source| ServeError::Io { source })?;
        self.wait()
    }

    /// Blocks until a core ends the service (`Goodbye` shutdown or a completed
    /// drain), then joins every thread.
    ///
    /// # Errors
    ///
    /// Returns [`ServeError::ServicePanicked`] or the holder shutdown failure.
    pub fn wait(self) -> Result<(), ServeError> {
        self.accept
            .join()
            .map_err(|_| ServeError::ServicePanicked)?
            .map_err(ServeError::from)
    }
}

fn accept_loop(context: &ServiceContext, listener: &UnixListener) -> Result<(), HolderError> {
    for stream in listener.incoming() {
        if context.stopping.load(Ordering::SeqCst) {
            break;
        }
        let failure = match stream {
            Ok(stream) => connection::serve(context, stream).err(),
            Err(source) => Some(ConnectionError::Io { source }),
        };
        if let Some(failure) = failure {
            *context
                .last_failure
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(failure);
        }
        if context.stopping.load(Ordering::SeqCst) {
            break;
        }
    }
    context.holder.shutdown()
}
