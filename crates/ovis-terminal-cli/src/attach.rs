//! `ovis-terminal terminal attach`: a plain attach client, not multiplexer chrome.
//!
//! Interact mode needs a TTY on stdin and stdout, holds the interact lease, puts the outer
//! terminal in raw mode for exactly as long as it is attached, forwards typed bytes only
//! while the stream is input-ready, follows window resizes under its geometry lease, and
//! detaches on Ctrl-\. Observe mode needs no TTY and never reads stdin. Every outcome,
//! including a refusal, a held lease, connection loss and interruption, leaves the
//! outer terminal exactly as it found it.

mod client;
mod open;
mod session;
mod tty;

use std::io;

use ovis_terminal_protocol::rpc::RpcErrorObject;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_server::listen::EndpointAddress;
use serde_json::Value;
use thiserror::Error;

/// One planned attach: everything was validated before a connection is opened.
#[derive(Debug, Clone)]
pub(crate) struct AttachPlan {
    pub(crate) target: Target,
    pub(crate) observe: bool,
    pub(crate) client: ClientId,
}

/// Connects, attaches and runs until detach, close or failure; the outer terminal is
/// restored before this returns. Returns the detach record.
pub(crate) async fn attach(
    plan: &AttachPlan,
    (address, bearer): (&EndpointAddress, Option<&str>),
) -> Result<Value, AttachError> {
    let EndpointAddress::Unix(path) = address else {
        return Err(AttachError::UnsupportedTransport);
    };
    let connected = client::Client::connect(path, bearer).await?;
    session::run(plan, connected).await
}

/// Why an attach ended without the user detaching.
#[derive(Debug, Error)]
pub(crate) enum AttachError {
    /// Interact mode was asked for without a terminal on stdin and stdout.
    #[error("interact attach needs a terminal on stdin and stdout")]
    NotATty,
    /// Another client holds the interact lease; nothing was taken over.
    #[error("another client holds the interact lease")]
    Held {
        /// The holder's lease record, as the server reported it.
        holder: Value,
    },
    /// The server refused the attach.
    #[error("the attach was refused: {reason}")]
    Refused {
        /// The server's reason token.
        reason: String,
    },
    /// The terminal is recovering; no stream was attached.
    #[error("the terminal is recovering")]
    Recovering,
    /// The terminal's program has ended.
    #[error("the terminal has ended: {reason}")]
    Ended {
        /// The server's reason token.
        reason: String,
    },
    /// More was typed than may be held while the stream is not input-ready.
    #[error("input was typed faster than the stream became input-ready")]
    InputNotReady,
    /// The connection ended while attached.
    #[error("the connection to the server was lost")]
    ConnectionLost,
    /// The server answered a request with a typed error.
    #[error("the server refused a request")]
    Server(Box<RpcErrorObject>),
    /// A socket or terminal operation failed.
    #[error("{operation} failed")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The server sent something this client cannot read.
    #[error("the server sent an unreadable message: {detail}")]
    Protocol {
        /// What was wrong.
        detail: &'static str,
    },
    /// Only Unix endpoints carry attach in this build.
    #[error("this endpoint has no attach transport in this build")]
    UnsupportedTransport,
}

impl AttachError {
    /// The stable machine-readable kind; a server refusal is reported as the server's own
    /// typed error instead, so its token here only names the class.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::NotATty => "not_a_tty",
            Self::Held { .. } => "lease_held",
            Self::Refused { .. } => "attach_refused",
            Self::Recovering => "recovering",
            Self::Ended { .. } => "finished",
            Self::InputNotReady => "not_ready",
            Self::ConnectionLost => "connection_lost",
            Self::Server(_) => "server_refused",
            Self::Io { .. } => "transport_failed",
            Self::Protocol { .. } => "invalid_response",
            Self::UnsupportedTransport => "unsupported_transport",
        }
    }

    /// Recovery guidance for the kind.
    pub(crate) const fn hint(&self) -> &'static str {
        match self {
            Self::NotATty => "run in a terminal, or pass --observe to watch without one",
            Self::Held { .. } => {
                "watch with --observe, or wait until the holder named in holder detaches"
            }
            Self::Refused { .. } => "check the credential's input grant for this terminal",
            Self::Recovering => "retry after the terminal recovers",
            Self::Ended { .. } => "read the final screen with --observe or terminal get",
            Self::InputNotReady => "attach again and type once the screen appears",
            Self::ConnectionLost => "check that the server is still running, then attach again",
            Self::Server(_) => "see the server's error kind and hint",
            Self::Io { .. } => "check the endpoint and the terminal device",
            Self::Protocol { .. } => "check that the endpoint belongs to this generation",
            Self::UnsupportedTransport => "attach over the Unix endpoint",
        }
    }
}
