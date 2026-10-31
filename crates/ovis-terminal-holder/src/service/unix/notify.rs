//! The systemd readiness datagram (`Type=notify`): the manager learns the holder is up
//! from the holder itself instead of a caller polling for it.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::net::{SocketAddr, UnixDatagram};

use super::ServiceError;

const fn io(source: std::io::Error) -> ServiceError {
    ServiceError::Io {
        operation: "notify the service manager",
        source,
    }
}

#[cfg(target_os = "linux")]
fn abstract_address(name: &[u8]) -> Result<SocketAddr, ServiceError> {
    <SocketAddr as std::os::linux::net::SocketAddrExt>::from_abstract_name(name).map_err(io)
}

/// Abstract socket names exist only on Linux.
#[cfg(not(target_os = "linux"))]
fn abstract_address(name: &[u8]) -> Result<SocketAddr, ServiceError> {
    Err(ServiceError::UnsupportedPath {
        path: std::path::PathBuf::from(OsStr::from_bytes(name)),
    })
}

/// Sends `READY=1` to the `NOTIFY_SOCKET` value `socket` (a path, or `@name` for an
/// abstract socket).
///
/// # Errors
///
/// The address cannot be formed or the datagram cannot be sent.
pub fn notify_ready(socket: &OsStr) -> Result<(), ServiceError> {
    let bytes = socket.as_bytes();
    let address = match bytes.strip_prefix(b"@") {
        Some(name) => abstract_address(name)?,
        None => SocketAddr::from_pathname(socket).map_err(io)?,
    };
    let sender = UnixDatagram::unbound().map_err(io)?;
    sender
        .send_to_addr(b"READY=1", &address)
        .map(drop)
        .map_err(io)
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixDatagram;

    use super::notify_ready;
    use crate::service::unix::test_root::TestRoot;

    #[test]
    fn the_manager_socket_receives_exactly_ready() {
        let root = TestRoot::new("notify");
        let path = root.path().join("notify");
        let manager = UnixDatagram::bind(&path).unwrap();
        notify_ready(path.as_os_str()).unwrap();
        let mut buffer = [0_u8; 32];
        let read = manager.recv(&mut buffer).unwrap();
        assert_eq!(buffer.get(..read), Some(&b"READY=1"[..]));
    }
}
