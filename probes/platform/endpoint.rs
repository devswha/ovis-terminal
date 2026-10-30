//! Endpoint identity: Unix socket path limits, ownership and replacement
//! detection, and Windows named-pipe first-instance ownership.

#[cfg(unix)]
#[path = "endpoint_unix.rs"]
mod os;

#[cfg(windows)]
#[path = "endpoint_windows.rs"]
mod os;

#[cfg(unix)]
pub use os::{
    BrokenEndpoint, EndpointReport, SocketIdentity, probe_broken_unix_endpoint, probe_unix_endpoint,
};
#[cfg(windows)]
pub use os::{PipeReport, probe_broken_named_pipe, probe_named_pipe};
