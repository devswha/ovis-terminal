//! Why an attach ended without the user detaching, with the typed facts each refusal
//! carries beyond its kind.

use std::io;

use ovis_terminal_protocol::rpc::RpcErrorObject;
use serde_json::Value;
use thiserror::Error;

/// Why an attach ended without the user detaching.
#[derive(Debug, Error)]
pub(crate) enum AttachError {
    /// Interact mode was asked for without a terminal on stdin and stdout.
    #[error("interact attach needs a terminal on stdin and stdout")]
    NotATty,
    /// Another client holds the interact lease; nothing was taken over or evicted.
    #[error("another client holds the interact lease")]
    Conflict {
        /// The holder's lease record, as the server reported it.
        holder: Value,
        /// The lease change to wait for before attaching again.
        retry: Value,
    },
    /// An explicit takeover named an epoch the interact lease no longer has.
    #[error("the takeover named epoch {expected}, which the interact lease no longer holds")]
    EpochMismatch {
        /// The epoch the caller presented, as a decimal string.
        expected: String,
        /// The interact lease holding the slot now, read after the refusal.
        current: Value,
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
    /// A signal arrived before the stream was attached; nothing is left attached.
    #[error("{signal} arrived while {stage}")]
    Interrupted {
        /// The stage the attach was in.
        stage: &'static str,
        /// The signal's name.
        signal: &'static str,
    },
    /// The outer terminal hung up while the screen was being drawn on it.
    #[error("the outer terminal hung up")]
    TerminalGone,
    /// More was typed than may be held while the stream is not input-ready.
    #[error("input was typed faster than the stream became input-ready")]
    InputNotReady,
    /// The connection ended while attached.
    #[error("the connection to the server was lost")]
    ConnectionLost {
        /// The server's `$/closing` record, when it announced the end first.
        closing: Option<Value>,
    },
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
            Self::Conflict { .. } => "lease_conflict",
            Self::EpochMismatch { .. } => "lease_epoch_mismatch",
            Self::Refused { .. } => "attach_refused",
            Self::Recovering => "recovering",
            Self::Ended { .. } => "finished",
            Self::Interrupted { .. } => "interrupted",
            Self::TerminalGone => "terminal_hung_up",
            Self::InputNotReady => "not_ready",
            Self::ConnectionLost { .. } => "connection_lost",
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
            Self::Conflict { .. } => {
                "watch with --observe, attach again after the retry event names the holder's lease, or pass --takeover with its epoch"
            }
            Self::EpochMismatch { .. } => {
                "repeat with --takeover set to the epoch in current, or attach without takeover"
            }
            Self::Refused { .. } => "check the credential's input grant for this terminal",
            Self::Recovering => "retry after the terminal recovers",
            Self::Ended { .. } => "read the final screen with --observe or terminal get",
            Self::Interrupted { .. } => "attach again; nothing was left attached",
            Self::TerminalGone => "attach again from a terminal that stays open",
            Self::InputNotReady => "attach again and type once the screen appears",
            Self::ConnectionLost { .. } => {
                "check that the server is still running, then attach again"
            }
            Self::Server(_) => "see the server's error kind and hint",
            Self::Io { .. } => "check the endpoint and the terminal device",
            Self::Protocol { .. } => "check that the endpoint belongs to this generation",
            Self::UnsupportedTransport => "attach over the Unix endpoint",
        }
    }

    /// The typed facts the error record carries beside kind and hint.
    pub(crate) fn details(&self) -> Vec<(&'static str, Value)> {
        match self {
            Self::Conflict { holder, retry } => {
                vec![("holder", holder.clone()), ("retry", retry.clone())]
            }
            Self::EpochMismatch { expected, current } => vec![
                ("expected_epoch", Value::from(expected.as_str())),
                ("current", current.clone()),
            ],
            Self::Interrupted { stage, signal } => vec![
                ("stage", Value::from(*stage)),
                ("signal", Value::from(*signal)),
            ],
            Self::Refused { reason } | Self::Ended { reason } => {
                vec![("reason", Value::from(reason.as_str()))]
            }
            Self::ConnectionLost {
                closing: Some(closing),
            } => vec![("closing", closing.clone())],
            Self::NotATty
            | Self::Recovering
            | Self::TerminalGone
            | Self::InputNotReady
            | Self::ConnectionLost { closing: None }
            | Self::Server(_)
            | Self::Io { .. }
            | Self::Protocol { .. }
            | Self::UnsupportedTransport => Vec::new(),
        }
    }
}
