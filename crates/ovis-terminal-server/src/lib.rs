//! Authenticated persistent endpoints, strict dispatch, streams and WebSocket transport.
//!
//! Scaffold crate: it compiles and records its place in the workspace layering;
//! runtime behavior arrives with the plan todos named on each module.

pub mod conn;
pub mod dispatch;
pub mod listen;
pub mod streams;
pub mod ws;

/// Whether a transport error only says the peer hung up, the normal end of a connection.
fn peer_gone(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
    )
}

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_holder::CRATE_NAME,
    ovis_terminal_core::CRATE_NAME,
    ovis_terminal_authz::CRATE_NAME,
    ovis_terminal_agent::CRATE_NAME,
    ovis_terminal_agent_adapters::CRATE_NAME,
    ovis_terminal_services::CRATE_NAME,
    ovis_terminal_remote::CRATE_NAME,
];
