//! The core side of a holder socket: framed send and bounded receive.

use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use thiserror::Error;

use crate::protocol::{FrameDecoder, FrameError, Message, encode_message};

/// A core-side holder connection failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreConnectionError {
    /// The socket failed.
    #[error("the holder socket failed")]
    Io {
        /// Socket failure.
        #[source]
        source: io::Error,
    },
    /// A frame could not be encoded or decoded.
    #[error("a holder frame was rejected")]
    Frame {
        /// Frame failure.
        #[from]
        source: FrameError,
    },
    /// No complete message arrived before the deadline.
    #[error("no holder message arrived before the deadline")]
    Timeout,
    /// The holder closed the connection between frames.
    #[error("the holder closed the connection")]
    Closed,
}

impl CoreConnectionError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Io { .. } => "holder_socket_io",
            Self::Frame { source } => source.kind(),
            Self::Timeout => "recv_timeout",
            Self::Closed => "holder_closed",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Io { .. } | Self::Closed => "Reconnect and re-adopt the holder.",
            Self::Frame { source } => source.hint(),
            Self::Timeout => "Wait again, or inspect the holder's retained records.",
        }
    }
}

/// A connection from a core to a holder socket.
#[derive(Debug)]
pub struct CoreConnection {
    stream: UnixStream,
    decoder: FrameDecoder,
    pending: VecDeque<Message>,
}

impl CoreConnection {
    /// Connects to the holder socket at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreConnectionError::Io`] when the socket cannot be reached.
    pub fn connect(path: &Path) -> Result<Self, CoreConnectionError> {
        let stream =
            UnixStream::connect(path).map_err(|source| CoreConnectionError::Io { source })?;
        Ok(Self {
            stream,
            decoder: FrameDecoder::default(),
            pending: VecDeque::new(),
        })
    }

    /// Sends one framed message.
    ///
    /// # Errors
    ///
    /// Returns a frame encoding or socket failure.
    pub fn send(&mut self, message: &Message) -> Result<(), CoreConnectionError> {
        let frame = encode_message(message)?;
        self.send_raw(&frame)
    }

    /// Sends bytes exactly as given, without framing them.
    ///
    /// # Errors
    ///
    /// Returns [`CoreConnectionError::Io`] when the write fails.
    pub fn send_raw(&mut self, bytes: &[u8]) -> Result<(), CoreConnectionError> {
        self.stream
            .write_all(bytes)
            .map_err(|source| CoreConnectionError::Io { source })
    }

    /// Waits at most `timeout` of socket inactivity for the next message.
    ///
    /// # Errors
    ///
    /// Returns [`CoreConnectionError::Timeout`], [`CoreConnectionError::Closed`],
    /// or a frame or socket failure.
    pub fn recv(&mut self, timeout: Duration) -> Result<Message, CoreConnectionError> {
        match self.stream.set_read_timeout(Some(timeout)) {
            Ok(()) => {}
            // macOS rejects socket options once the peer shut the socket down; the
            // read below then reports the end of the connection.
            Err(error) if error.raw_os_error() == Some(libc::EINVAL) => {}
            Err(source) => return Err(CoreConnectionError::Io { source }),
        }
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            if let Some(message) = self.pending.pop_front() {
                return Ok(message);
            }
            let read = match self.stream.read(&mut buffer) {
                Ok(0) => {
                    self.decoder.finish()?;
                    return Err(CoreConnectionError::Closed);
                }
                Ok(read) => read,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return Err(CoreConnectionError::Timeout);
                }
                Err(source) => return Err(CoreConnectionError::Io { source }),
            };
            let chunk = buffer.get(..read).unwrap_or_default();
            self.pending.extend(self.decoder.push(chunk)?);
        }
    }
}
