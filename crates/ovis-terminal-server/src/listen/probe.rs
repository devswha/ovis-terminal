//! Side-effect-free endpoint status: running, stopped, unreachable or stale.
//!
//! A probe only inspects the path and, when a socket answers, sends one `hello`. It never
//! creates, binds, removes or starts anything, so observing a stopped runtime leaves it
//! stopped.

use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use ovis_terminal_protocol::rpc::{RpcReply, parse_reply};
use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use ovis_terminal_protocol::schema::methods::server::{HelloResult, ServerState};
use ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::EndpointError;
use super::unix::{PathOccupant, io_error, occupant};

/// The observed state of an endpoint path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndpointStatus {
    /// A server answered hello; its live identity is attached.
    Running(Box<HelloResult>),
    /// Nothing exists at the path.
    Stopped,
    /// A socket accepts connections but no hello result arrived in time.
    Unreachable,
    /// The path holds a socket nobody listens on, or a file that is not a socket.
    StaleEndpoint,
}

impl EndpointStatus {
    /// The protocol state this observation reports.
    #[must_use]
    pub const fn state(&self) -> ServerState {
        match self {
            Self::Running(_) => ServerState::Running,
            Self::Stopped => ServerState::Stopped,
            Self::Unreachable => ServerState::Unreachable,
            Self::StaleEndpoint => ServerState::StaleEndpoint,
        }
    }
}

/// The credential a probe authenticates with and how long it waits for hello.
#[derive(Debug, Clone, Copy)]
pub struct Probe<'a> {
    bearer: &'a str,
    deadline: Duration,
}

impl<'a> Probe<'a> {
    /// Probes with `bearer`, waiting at most `deadline` for the hello result.
    #[must_use]
    pub const fn new(bearer: &'a str, deadline: Duration) -> Self {
        Self { bearer, deadline }
    }

    /// Observes the endpoint at `path`.
    ///
    /// # Errors
    ///
    /// [`EndpointError::HelloRejected`] when a live server refuses the hello,
    /// [`EndpointError::MalformedReply`] for a reply that is not a hello result and
    /// [`EndpointError::Io`] when the path cannot be inspected.
    pub async fn observe(self, path: &Path) -> Result<EndpointStatus, EndpointError> {
        match occupant(path)? {
            PathOccupant::Vacant => return Ok(EndpointStatus::Stopped),
            PathOccupant::Other => return Ok(EndpointStatus::StaleEndpoint),
            PathOccupant::Socket => {}
        }
        let stream = match UnixStream::connect(path).await {
            Ok(stream) => stream,
            Err(error) => {
                return match error.kind() {
                    ErrorKind::ConnectionRefused => Ok(EndpointStatus::StaleEndpoint),
                    ErrorKind::NotFound => Ok(EndpointStatus::Stopped),
                    _ => Err(io_error("connect", path, error)),
                };
            }
        };
        let Ok(exchanged) = tokio::time::timeout(self.deadline, self.hello(stream)).await else {
            return Ok(EndpointStatus::Unreachable);
        };
        let Some(line) = exchanged.map_err(|source| io_error("hello", path, source))? else {
            return Ok(EndpointStatus::Unreachable);
        };
        match parse_reply(&line).map_err(EndpointError::MalformedReply)? {
            RpcReply::Success(success) => serde_json::from_value(success.result)
                .map(|result| EndpointStatus::Running(Box::new(result)))
                .map_err(EndpointError::Identity),
            RpcReply::Failure(failure) => Err(EndpointError::HelloRejected {
                error: Box::new(failure.error),
            }),
            _ => Err(EndpointError::MalformedReply(
                ovis_terminal_protocol::errors::BoundaryError::invalid_response(),
            )),
        }
    }

    async fn hello(self, mut stream: UnixStream) -> std::io::Result<Option<String>> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "status-probe",
            "method": "hello",
            "params": {
                "generation": PROTOCOL_GENERATION,
                "client": { "name": "ovis-terminal-probe", "version": env!("CARGO_PKG_VERSION") },
                "credential": self.bearer,
            },
        });
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        stream.write_all(&line).await?;
        let limit = u64::try_from(MAX_FRAME_PAYLOAD).map_err(std::io::Error::other)?;
        let mut reader = BufReader::new(stream.take(limit));
        let mut reply = String::new();
        let read = reader.read_line(&mut reply).await?;
        Ok((read > 0 && reply.ends_with('\n')).then_some(reply))
    }
}
