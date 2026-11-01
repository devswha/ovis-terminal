//! Authenticated remote machines over byte-transparent transports.
//!
//! Machines are keyed by the identity their runtime proves over a fresh challenge
//! ([`identity`]) and reached over transports that carry protocol bytes unchanged
//! ([`transport`]). Each machine has its own actor ([`actors`]); its events are mirrored
//! into the local sequence domain ([`mirror`]) and parent deaths reach it as orphan notices
//! kept until it acknowledges them ([`orphans`]).

pub mod actors;
pub mod identity;
pub mod mirror;
pub mod orphans;
pub mod transport;

#[cfg(test)]
pub mod qa_support;

use actors::Line;
use transport::TransportError;

/// One actor table over two kinds of line, such as spawned transports beside in-process
/// or socket lines: each machine keeps whichever line reached it.
#[derive(Debug)]
#[non_exhaustive]
pub enum EitherLine<A, B> {
    /// A line of the first kind.
    First(A),
    /// A line of the second kind.
    Second(B),
}

impl<A: Line, B: Line> Line for EitherLine<A, B> {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        match self {
            Self::First(line) => line.send(bytes).await,
            Self::Second(line) => line.send(bytes).await,
        }
    }

    async fn receive(&mut self) -> Result<Vec<u8>, TransportError> {
        match self {
            Self::First(line) => line.receive().await,
            Self::Second(line) => line.receive().await,
        }
    }
}

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[
    ovis_terminal_protocol::CRATE_NAME,
    ovis_terminal_core::CRATE_NAME,
    ovis_terminal_authz::CRATE_NAME,
    ovis_terminal_store::CRATE_NAME,
];
