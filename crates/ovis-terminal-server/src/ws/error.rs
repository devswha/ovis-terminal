//! The WebSocket transport boundary error, its stable kinds and recovery hints.

use std::{fmt, io};

use thiserror::Error;

use crate::conn::ConnectionError;

/// Why a WebSocket listener or one of its connections stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WsError {
    /// The listen address could not be bound or reported.
    #[error("the WebSocket listen address could not be bound")]
    Bind(#[source] io::Error),
    /// The listener could not accept a connection.
    #[error("the WebSocket listener could not accept")]
    Accept(#[source] io::Error),
    /// A configured origin is not `http(s)://host[:port]`.
    #[error("a configured origin is not scheme://host[:port] over http or https")]
    InvalidOrigin,
    /// The upgrade request came from a browser origin that is not configured.
    #[error("the upgrade request's origin is not allowed")]
    OriginRejected,
    /// The peer did not complete the upgrade request in time.
    #[error("the upgrade request did not arrive within {after_ms} ms")]
    HandshakeTimeout {
        /// The handshake deadline.
        after_ms: u128,
    },
    /// The upgrade request is not a WebSocket opening handshake.
    #[error("the upgrade request is not a WebSocket opening handshake: {0}")]
    InvalidUpgrade(UpgradeDefect),
    /// The upgrade request asks for a WebSocket version other than 13.
    #[error("the upgrade request asks for a WebSocket version other than 13")]
    UnsupportedVersion,
    /// The upgrade request head exceeds its byte or header bound.
    #[error("the upgrade request head exceeds its bound")]
    UpgradeTooLarge,
    /// The peer sent a text message; the protocol travels in binary messages only.
    #[error("the peer sent a text message")]
    TextMessage,
    /// A frame declared more payload than the protocol's frame bound.
    #[error("a frame declared {length} payload bytes, beyond the frame bound")]
    MessageTooLarge {
        /// The declared payload length.
        length: usize,
    },
    /// The peer violated RFC 6455 framing.
    #[error("the peer violated WebSocket framing: {0}")]
    Protocol(FrameDefect),
    /// Reading or writing the TCP stream failed.
    #[error("the WebSocket transport failed")]
    Transport(#[source] io::Error),
    /// The connection core closed the connection.
    #[error("the connection was closed")]
    Connection(#[source] ConnectionError),
    /// The served handler or identity could not form a server.
    #[error("the server could not be formed")]
    Endpoint(#[source] crate::listen::EndpointError),
    /// The server task ended by panicking or being cancelled.
    #[error("the server task did not finish normally")]
    ServerTask(#[source] tokio::task::JoinError),
}

/// How a client frame violates RFC 6455.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameDefect {
    /// The frame is not masked.
    Unmasked,
    /// An extension bit is set; no extension was negotiated.
    ReservedBits,
    /// The opcode is not defined.
    UnknownOpcode,
    /// A control frame is fragmented or longer than 125 bytes.
    Control,
    /// A continuation frame arrived with no fragmented message open.
    UnexpectedContinuation,
    /// A new data frame arrived inside a fragmented message.
    InterleavedMessage,
    /// A close frame carries a one-byte payload.
    CloseLength,
}

impl fmt::Display for FrameDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unmasked => "an unmasked client frame",
            Self::ReservedBits => "a reserved bit is set",
            Self::UnknownOpcode => "an undefined opcode",
            Self::Control => "a fragmented or oversized control frame",
            Self::UnexpectedContinuation => "a continuation without a message",
            Self::InterleavedMessage => "a data frame inside a fragmented message",
            Self::CloseLength => "a one-byte close payload",
        })
    }
}

/// What is wrong with an upgrade request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpgradeDefect {
    /// The peer closed before sending a whole request head.
    Truncated,
    /// The head is not HTTP/1.x.
    Malformed,
    /// The request is not `GET` over HTTP/1.1.
    NotGet,
    /// `Upgrade: websocket` or `Connection: Upgrade` is missing.
    NotUpgrade,
    /// `Sec-WebSocket-Key` is missing, repeated or not 16 base64-encoded bytes.
    Key,
}

impl fmt::Display for UpgradeDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Truncated => "the request head ended early",
            Self::Malformed => "the head is not HTTP",
            Self::NotGet => "the request is not GET over HTTP/1.1",
            Self::NotUpgrade => "the upgrade headers are missing",
            Self::Key => "the key is missing or malformed",
        })
    }
}

impl WsError {
    /// The stable machine-readable kind, also the reason of a close frame the server sends.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Bind(_) => "bind_failed",
            Self::Accept(_) => "accept_failed",
            Self::InvalidOrigin => "invalid_origin",
            Self::OriginRejected => "origin_rejected",
            Self::HandshakeTimeout { .. } => "handshake_timeout",
            Self::InvalidUpgrade(_) => "invalid_upgrade",
            Self::UnsupportedVersion => "unsupported_ws_version",
            Self::UpgradeTooLarge => "upgrade_too_large",
            Self::TextMessage => "invalid_ws_message",
            Self::MessageTooLarge { .. } => "message_too_large",
            Self::Protocol(_) => "ws_protocol",
            Self::Transport(_) => "connection_io",
            Self::Connection(error) => error.kind(),
            Self::Endpoint(error) => error.kind(),
            Self::ServerTask(_) => "internal",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Bind(_) => "choose a free address or stop the process that holds it",
            Self::Accept(_) => "check free descriptors, then restart the listener",
            Self::InvalidOrigin => "configure origins as http(s)://host[:port] without a path",
            Self::OriginRejected => "connect from a configured origin or add it to the server",
            Self::HandshakeTimeout { .. } => "send the upgrade request right after connecting",
            Self::InvalidUpgrade(_) => "send an HTTP/1.1 GET upgrade with a Sec-WebSocket-Key",
            Self::UnsupportedVersion => "use WebSocket version 13",
            Self::UpgradeTooLarge => "send an upgrade request head of at most 8 KiB",
            Self::TextMessage => "send the protocol's NDJSON or frames as binary messages",
            Self::MessageTooLarge { .. } => "keep each frame within limits.max_frame_bytes",
            Self::Protocol(_) => "use a client that follows RFC 6455",
            Self::Transport(_) => "reconnect",
            Self::Connection(error) => error.hint(),
            Self::Endpoint(error) => error.hint(),
            Self::ServerTask(_) => "inspect the runtime log and restart the server",
        }
    }

    /// The RFC 6455 status code a close frame for this failure carries.
    pub(super) const fn close_code(&self) -> u16 {
        match self {
            Self::Protocol(_) => 1002,
            Self::TextMessage => 1003,
            Self::MessageTooLarge { .. } => 1009,
            _ => 1011,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameDefect, UpgradeDefect, WsError};

    #[test]
    fn violations_carry_their_rfc_close_code() {
        let codes = [
            (WsError::TextMessage, 1003, "invalid_ws_message"),
            (
                WsError::MessageTooLarge { length: 2 },
                1009,
                "message_too_large",
            ),
            (
                WsError::Protocol(FrameDefect::Unmasked),
                1002,
                "ws_protocol",
            ),
            (
                WsError::InvalidUpgrade(UpgradeDefect::Key),
                1011,
                "invalid_upgrade",
            ),
        ];
        for (error, code, kind) in codes {
            assert_eq!((error.close_code(), error.kind()), (code, kind));
            assert!(
                !error.to_string().contains(kind),
                "Display never repeats the kind"
            );
        }
    }
}
