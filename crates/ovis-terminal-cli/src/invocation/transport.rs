//! The client transport: one persistent NDJSON connection per invocation, over the Unix
//! socket or the Windows named pipe the endpoint descriptor names.
//!
//! Connecting never creates the endpoint: a missing one is `stopped`, a refused one is
//! `stale_endpoint`, and both are reported instead of starting a server.

use std::io::ErrorKind;
use std::path::PathBuf;

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::{RequestId, RpcReply, RpcRequest, parse_reply, parse_request};
use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use ovis_terminal_protocol::schema::methods::server::HelloResult;
use ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;
use ovis_terminal_server::listen::EndpointAddress;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf,
    WriteHalf,
};

use super::context::Credential;
use super::error::CliError;
use super::report::Failure;

/// A connected byte stream of either platform transport.
trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

type Stream = Box<dyn Duplex>;

fn refused(endpoint: PathBuf, source: std::io::Error) -> CliError {
    match source.kind() {
        ErrorKind::NotFound => CliError::NotRunning {
            path: endpoint,
            state: "stopped",
        },
        ErrorKind::ConnectionRefused => CliError::NotRunning {
            path: endpoint,
            state: "stale_endpoint",
        },
        _ => CliError::Io {
            operation: "connect",
            source,
        },
    }
}

#[cfg(unix)]
async fn connect(address: &EndpointAddress) -> Result<Stream, CliError> {
    match address {
        EndpointAddress::Unix(path) => tokio::net::UnixStream::connect(path)
            .await
            .map(|stream| Box::new(stream) as Stream)
            .map_err(|source| refused(path.clone(), source)),
        _ => Err(CliError::UnsupportedTransport),
    }
}

/// Opening a named-pipe client completes immediately, so the future is already ready.
#[cfg(windows)]
fn connect(address: &EndpointAddress) -> std::future::Ready<Result<Stream, CliError>> {
    std::future::ready(match address {
        EndpointAddress::Pipe(name) => tokio::net::windows::named_pipe::ClientOptions::new()
            .open(name)
            .map(|pipe| Box::new(pipe) as Stream)
            .map_err(|source| refused(PathBuf::from(name), source)),
        _ => Err(CliError::UnsupportedTransport),
    })
}

fn hello_request(credential: Option<&Credential>) -> Result<RpcRequest, CliError> {
    let mut params = json!({
        "generation": PROTOCOL_GENERATION,
        "client": { "name": crate::BINARY_NAME, "version": crate::VERSION },
    });
    if let (Some(credential), Value::Object(fields)) = (credential, &mut params) {
        fields.insert("credential".to_owned(), json!(credential.bearer()));
    }
    let envelope = json!({ "jsonrpc": "2.0", "id": "hello", "method": "hello", "params": params });
    parse_request(&envelope.to_string()).map_err(CliError::Reply)
}

/// An authenticated connection after a successful hello.
pub(super) struct Session {
    wire: Wire,
    hello: HelloResult,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("hello", &self.hello)
            .finish_non_exhaustive()
    }
}

/// The two halves of one NDJSON connection.
struct Wire {
    reader: BufReader<ReadHalf<Stream>>,
    writer: WriteHalf<Stream>,
}

impl Session {
    /// Connects and completes hello; a refused hello is the server's typed error.
    pub(super) async fn open(
        address: &EndpointAddress,
        credential: Option<&Credential>,
    ) -> Result<Self, Failure> {
        let (read, writer) = tokio::io::split(connect(address).await?);
        let mut wire = Wire {
            reader: BufReader::new(read),
            writer,
        };
        let result = wire.call(&hello_request(credential)?).await?;
        let hello = serde_json::from_value(result).map_err(CliError::Result)?;
        Ok(Self { wire, hello })
    }

    /// The identity the server reported in hello.
    pub(super) const fn hello(&self) -> &HelloResult {
        &self.hello
    }

    /// Sends `request` and waits for the reply that settles its id.
    pub(super) async fn call(&mut self, request: &RpcRequest) -> Result<Value, Failure> {
        self.wire.call(request).await
    }
}

impl Wire {
    async fn call(&mut self, request: &RpcRequest) -> Result<Value, Failure> {
        let mut line = serde_json::to_vec(request).map_err(CliError::Encode)?;
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .await
            .map_err(|source| CliError::Io {
                operation: "send",
                source,
            })?;
        self.settle(request.id()).await
    }

    async fn settle(&mut self, id: &RequestId) -> Result<Value, Failure> {
        let limit = u64::try_from(MAX_FRAME_PAYLOAD).map_err(|_| CliError::Closed)?;
        let mut restarting = false;
        loop {
            let mut line = String::new();
            let read = (&mut self.reader)
                .take(limit)
                .read_line(&mut line)
                .await
                .map_err(|source| CliError::Io {
                    operation: "receive",
                    source,
                })?;
            if read == 0 || !line.ends_with('\n') {
                let closed = if restarting {
                    CliError::Restarted
                } else {
                    CliError::Closed
                };
                return Err(closed.into());
            }
            let value: Value = serde_json::from_str(&line)
                .map_err(|source| CliError::Reply(BoundaryError::invalid_response_json(source)))?;
            if value.get("id").is_none() {
                restarting |= value.get("method").and_then(Value::as_str) == Some("$/closing")
                    && value.pointer("/params/reason").and_then(Value::as_str)
                        == Some("server_restart");
                continue;
            }
            return match parse_reply(&line).map_err(CliError::Reply)? {
                reply if reply.id() != id => {
                    Err(CliError::Reply(BoundaryError::response_id_mismatch()).into())
                }
                RpcReply::Success(success) => Ok(success.result),
                RpcReply::Failure(failure) => Err(Failure::Server(Box::new(failure.error))),
                _ => Err(CliError::Reply(BoundaryError::invalid_response()).into()),
            };
        }
    }
}

/// Copies stdin to the endpoint and the endpoint to stdout until the server closes.
pub(super) async fn bridge(address: &EndpointAddress) -> Result<(), CliError> {
    let (mut read, mut write) = tokio::io::split(connect(address).await?);
    let io = |operation| move |source| CliError::Io { operation, source };
    let mut stdout = tokio::io::stdout();
    {
        let upstream = async {
            tokio::io::copy(&mut tokio::io::stdin(), &mut write).await?;
            write.shutdown().await
        };
        let downstream = tokio::io::copy(&mut read, &mut stdout);
        tokio::pin!(upstream, downstream);
        tokio::select! {
            copied = &mut downstream => copied.map_err(io("relay from server"))?,
            sent = &mut upstream => {
                sent.map_err(io("relay to server"))?;
                downstream.await.map_err(io("relay from server"))?
            }
        };
    }
    stdout.flush().await.map_err(io("flush stdout"))
}
