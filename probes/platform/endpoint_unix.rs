//! Unix domain socket identity probes.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use serde::Serialize;

use crate::platform::error::{EndpointOperation, ProbeError};
use crate::platform::measure::ByteCount;

const PING: &[u8] = b"ovis-probe-ping";
/// Socket paths are probed up to this many bytes; every Unix `sun_path` is shorter.
const LONGEST_PROBED_PATH: usize = 512;

/// Filesystem identity of a socket path; a replaced socket has another identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SocketIdentity {
    /// Device holding the socket inode.
    #[serde(with = "crate::platform::decimal")]
    pub device: u64,
    /// Socket inode.
    #[serde(with = "crate::platform::decimal")]
    pub inode: u64,
    /// Owning user id.
    pub owner_uid: u32,
    /// Permission bits.
    pub mode: u32,
}

/// Measured Unix socket endpoint behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct EndpointReport {
    /// Longest socket path, in bytes, that binding accepted.
    pub max_path_bytes: usize,
    /// Shortest socket path that binding rejected.
    pub first_rejected_path_bytes: usize,
    /// Identity of the first bound socket.
    pub identity: SocketIdentity,
    /// Identity after the socket was removed and bound again.
    pub rebound_identity: SocketIdentity,
    /// Owner of the directory the probe created, for comparison with the socket owner.
    pub directory_owner_uid: u32,
    /// Bytes sent by the client that the server read back unchanged.
    pub exchanged: ByteCount,
}

/// Typed failures observed on a broken endpoint.
#[derive(Debug)]
#[non_exhaustive]
pub struct BrokenEndpoint {
    /// Connecting after the socket file was removed.
    pub connect_to_removed: Result<(), ProbeError>,
    /// Writing after the peer closed its side.
    pub write_to_closed_peer: Result<(), ProbeError>,
}

fn endpoint_error(operation: EndpointOperation) -> impl Fn(io::Error) -> ProbeError {
    move |source| ProbeError::Endpoint { operation, source }
}

fn identity(path: &Path) -> Result<SocketIdentity, ProbeError> {
    let metadata =
        fs::symlink_metadata(path).map_err(endpoint_error(EndpointOperation::Inspect))?;
    if !metadata.file_type().is_socket() {
        let not_socket = io::Error::new(
            io::ErrorKind::InvalidData,
            "the endpoint path is not a socket",
        );
        return Err(endpoint_error(EndpointOperation::Inspect)(not_socket));
    }
    Ok(SocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner_uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
    })
}

fn bind_owner_only(path: &Path) -> Result<UnixListener, ProbeError> {
    let listener = UnixListener::bind(path).map_err(endpoint_error(EndpointOperation::Bind))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(endpoint_error(EndpointOperation::Bind))?;
    Ok(listener)
}

fn exchange(listener: &UnixListener, path: &Path) -> Result<ByteCount, ProbeError> {
    let mut client =
        UnixStream::connect(path).map_err(endpoint_error(EndpointOperation::Connect))?;
    let (mut server, _peer) = listener
        .accept()
        .map_err(endpoint_error(EndpointOperation::Accept))?;
    client
        .write_all(PING)
        .map_err(endpoint_error(EndpointOperation::Exchange))?;
    let mut echoed = [0_u8; PING.len()];
    server
        .read_exact(&mut echoed)
        .map_err(endpoint_error(EndpointOperation::Exchange))?;
    if echoed != PING {
        let corrupted = io::Error::new(io::ErrorKind::InvalidData, "the socket changed the bytes");
        return Err(endpoint_error(EndpointOperation::Exchange)(corrupted));
    }
    Ok(ByteCount::new(
        u64::try_from(PING.len()).unwrap_or(u64::MAX),
    ))
}

/// Measures the socket path limit by binding ever longer paths in `dir`.
fn path_limit(dir: &Path) -> Result<(usize, usize), ProbeError> {
    let base = dir.as_os_str().len().saturating_add(1);
    let mut longest = 0;
    for name_len in 1..LONGEST_PROBED_PATH.saturating_sub(base) {
        let path = dir.join("p".repeat(name_len));
        match UnixListener::bind(&path) {
            Ok(_listener) => {
                fs::remove_file(&path).map_err(endpoint_error(EndpointOperation::Bind))?;
                longest = base.saturating_add(name_len);
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                return Ok((longest, base.saturating_add(name_len)));
            }
            Err(error) => return Err(endpoint_error(EndpointOperation::Bind)(error)),
        }
    }
    let unbounded = io::Error::other("no socket path length was rejected");
    Err(endpoint_error(EndpointOperation::Bind)(unbounded))
}

/// Binds, exchanges bytes over, removes and rebinds a socket in `dir`, and
/// measures the longest accepted socket path there.
///
/// # Errors
///
/// [`ProbeError::Endpoint`] naming the failed operation.
pub fn probe_unix_endpoint(dir: &Path) -> Result<EndpointReport, ProbeError> {
    let (max_path_bytes, first_rejected_path_bytes) = path_limit(dir)?;
    let path = dir.join("ovis-probe.sock");
    let listener = bind_owner_only(&path)?;
    let identity = identity(&path)?;
    let exchanged = exchange(&listener, &path)?;
    drop(listener);
    fs::remove_file(&path).map_err(endpoint_error(EndpointOperation::Bind))?;
    let rebound = bind_owner_only(&path)?;
    let rebound_identity = self::identity(&path)?;
    drop(rebound);
    fs::remove_file(&path).map_err(endpoint_error(EndpointOperation::Bind))?;
    let directory_owner_uid = fs::metadata(dir)
        .map_err(endpoint_error(EndpointOperation::Inspect))?
        .uid();
    Ok(EndpointReport {
        max_path_bytes,
        first_rejected_path_bytes,
        identity,
        rebound_identity,
        directory_owner_uid,
        exchanged,
    })
}

/// Connects to a removed socket and writes to a closed peer in `dir`.
///
/// # Errors
///
/// [`ProbeError::Endpoint`] when the broken endpoint cannot be set up.
pub fn probe_broken_unix_endpoint(dir: &Path) -> Result<BrokenEndpoint, ProbeError> {
    let path = dir.join("ovis-probe-broken.sock");
    let listener = bind_owner_only(&path)?;
    let mut client =
        UnixStream::connect(&path).map_err(endpoint_error(EndpointOperation::Connect))?;
    let (server, _peer) = listener
        .accept()
        .map_err(endpoint_error(EndpointOperation::Accept))?;
    drop(server);
    drop(listener);
    fs::remove_file(&path).map_err(endpoint_error(EndpointOperation::Bind))?;
    let connect_to_removed = UnixStream::connect(&path)
        .map(drop)
        .map_err(endpoint_error(EndpointOperation::Connect));
    let write_to_closed_peer = client
        .write_all(PING)
        .map_err(endpoint_error(EndpointOperation::Exchange));
    Ok(BrokenEndpoint {
        connect_to_removed,
        write_to_closed_peer,
    })
}
