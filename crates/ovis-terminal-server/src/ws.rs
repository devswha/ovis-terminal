//! Authenticated WebSocket transport with protocol parity (plan todo 28).
//!
//! A WebSocket connection is one more byte stream for [`crate::conn::Connections`]: the
//! payloads of the client's binary messages are concatenated in order and decoded by the
//! same connection core, so hello, the credential check, limits, framing, dispatch and
//! streams are exactly those of the local socket. Every record the server writes (one
//! NDJSON line, or one frame after a framed hello) leaves as exactly one binary message.
//!
//! The upgrade request is checked before anything else runs: a request carrying an
//! `Origin` header is refused with HTTP 403 unless that origin is configured. Requests
//! without `Origin` come from non-browser clients and authenticate like any socket peer,
//! with `hello.credential`. Text messages are not part of the protocol and close the
//! connection. The default binding is the loopback interface.

mod binding;
mod error;
mod frame;
mod handle;
mod origin;
mod pump;
mod server;
mod transport;
mod upgrade;

pub use binding::{WsBinding, WsEndpoint};
pub use error::{FrameDefect, UpgradeDefect, WsError};
pub use handle::{WsReport, WsServerHandle};
pub use origin::{AllowedOrigins, Origin};
pub use server::{WsConfig, serve_websocket};
