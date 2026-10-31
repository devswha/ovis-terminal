//! Authenticated persistent endpoint listeners (plan todo 13).
//!
//! Descriptors, including the Windows pipe name, and identity continuity are portable.
//! Serving and probing a Unix socket is Unix-only; other listeners hand their accepted
//! streams to [`crate::conn::Connections`].

mod continuity;
mod descriptor;
mod error;
#[cfg(unix)]
mod probe;
#[cfg(unix)]
mod server;
#[cfg(unix)]
mod unix;

pub use continuity::{Continuity, EndpointIdentity};
pub use descriptor::{
    AddressSource, EndpointAddress, EndpointDescriptor, EndpointEnvironment, EnvVar,
    MAX_UNIX_SOCKET_PATH, PIPE_PREFIX, Platform, SessionName,
};
pub use error::EndpointError;
#[cfg(unix)]
pub use probe::{EndpointStatus, Probe};
#[cfg(unix)]
pub use server::{ServeReport, ServerConfig, ServerHandle, serve_unix};
#[cfg(unix)]
pub use unix::UnixEndpoint;
