//! Windows named-pipe identity probes (CI smoke tier).

use std::io;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
use tokio::runtime::{Builder, Runtime};

use crate::platform::error::{EndpointOperation, ProbeError};
use crate::platform::measure::ByteCount;

const PING: &[u8] = b"ovis-probe-ping";

/// Measured named-pipe behavior.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct PipeReport {
    /// A second first-instance server on the same name was refused.
    pub first_instance_exclusive: bool,
    /// OS error kind of that refusal.
    pub second_instance_error: String,
    /// Bytes sent by the client that the server read unchanged.
    pub exchanged: ByteCount,
}

fn endpoint_error(operation: EndpointOperation) -> impl Fn(io::Error) -> ProbeError {
    move |source| ProbeError::Endpoint { operation, source }
}

fn runtime() -> Result<Runtime, ProbeError> {
    Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(endpoint_error(EndpointOperation::Bind))
}

/// Creates `\\.\pipe\<name>` as the first instance, checks that a second first
/// instance is refused and exchanges bytes with a client.
///
/// # Errors
///
/// [`ProbeError::Endpoint`] naming the failed operation.
pub fn probe_named_pipe(name: &str) -> Result<PipeReport, ProbeError> {
    let address = format!(r"\\.\pipe\{name}");
    runtime()?.block_on(async {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&address)
            .map_err(endpoint_error(EndpointOperation::Bind))?;
        let second = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&address);
        let second_instance_error = second
            .as_ref()
            .err()
            .map_or_else(String::new, |error| error.kind().to_string());
        let first_instance_exclusive = second.is_err();
        drop(second);
        let mut client = ClientOptions::new()
            .open(&address)
            .map_err(endpoint_error(EndpointOperation::Connect))?;
        let mut server = server;
        server
            .connect()
            .await
            .map_err(endpoint_error(EndpointOperation::Accept))?;
        client
            .write_all(PING)
            .await
            .map_err(endpoint_error(EndpointOperation::Exchange))?;
        let mut received = [0_u8; PING.len()];
        server
            .read_exact(&mut received)
            .await
            .map_err(endpoint_error(EndpointOperation::Exchange))?;
        if received != PING {
            let corrupted =
                io::Error::new(io::ErrorKind::InvalidData, "the pipe changed the bytes");
            return Err(endpoint_error(EndpointOperation::Exchange)(corrupted));
        }
        Ok(PipeReport {
            first_instance_exclusive,
            second_instance_error,
            exchanged: ByteCount::new(u64::try_from(PING.len()).unwrap_or(u64::MAX)),
        })
    })
}

/// Opens a client on a pipe name that no server created.
///
/// # Errors
///
/// [`ProbeError::Endpoint`] when the runtime cannot start; the client result
/// itself is returned for the caller to inspect.
pub fn probe_broken_named_pipe(name: &str) -> Result<Result<(), ProbeError>, ProbeError> {
    let address = format!(r"\\.\pipe\{name}");
    let runtime = runtime()?;
    let entered = runtime.enter();
    let opened = ClientOptions::new()
        .open(&address)
        .map(drop)
        .map_err(endpoint_error(EndpointOperation::Connect));
    drop(entered);
    Ok(opened)
}
