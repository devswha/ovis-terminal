//! Failures of a transport before or during the handshake.

use std::time::Duration;

/// Bytes a peer sent, kept as evidence and shown escaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence(Vec<u8>);

impl Evidence {
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }

    /// The raw bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Display for Evidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.escape_ascii())
    }
}

/// Why a transport could not carry the handshake to a verified reply.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The transport specification is unsafe or unknown.
    #[error("the transport specification is refused: {reason}")]
    InvalidSpec {
        /// What is wrong with it.
        reason: &'static str,
    },
    /// The transport program could not be started.
    #[error("the transport program {program} could not start")]
    Spawn {
        /// The program.
        program: String,
        /// The operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// Reading or writing the transport's standard streams failed.
    #[error("the transport's standard streams failed")]
    Io(#[from] std::io::Error),
    /// The transport ended before the reply; stderr is kept as diagnostics.
    #[error(
        "the transport ended before the handshake reply (exit status {status:?}; stderr: {diagnostics})"
    )]
    Closed {
        /// The exit code, when the program exited with one.
        status: Option<i32>,
        /// What the program wrote to stderr, bounded.
        diagnostics: Evidence,
    },
    /// The first stdout bytes are not the hello reply, such as a banner or a prompt.
    #[error("the transport wrote bytes that are not a protocol reply: {first_bytes}")]
    NotProtocol {
        /// The offending bytes, bounded.
        first_bytes: Evidence,
    },
    /// The handshake did not complete within its bound.
    #[error("no handshake reply within {bound:?}; received: {received}")]
    Timeout {
        /// The bound.
        bound: Duration,
        /// Bytes of an incomplete reply, such as a credential prompt.
        received: Evidence,
    },
    /// The remote runtime refused the hello with a typed error.
    #[error("the remote runtime refused the hello ({remote_kind})")]
    Refused {
        /// The remote error kind.
        remote_kind: String,
    },
    /// The hello reply does not decode as a result of this protocol.
    #[error("the remote hello result does not match this protocol")]
    Incompatible(#[source] serde_json::Error),
}

impl TransportError {
    /// Stable machine token.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidSpec { .. } => "invalid_params",
            Self::Spawn { .. } | Self::Io(_) | Self::Closed { .. } | Self::NotProtocol { .. } => {
                "transport_failed"
            }
            Self::Timeout { .. } => "handshake_timeout",
            Self::Refused { remote_kind } => match remote_kind.as_str() {
                "unauthenticated" | "invalid_credential" | "credential_expired"
                | "credential_revoked" => "auth_failed",
                "unsupported_generation" => "remote_incompatible",
                _ => "transport_failed",
            },
            Self::Incompatible(_) => "remote_incompatible",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub fn hint(&self) -> &'static str {
        match self.kind() {
            "invalid_params" => "pass [user@]host or a program and its arguments",
            "handshake_timeout" => "make the transport non-interactive and the runtime reachable",
            "auth_failed" => "issue a fresh credential on the remote runtime",
            "remote_incompatible" => "update the remote runtime to this protocol generation",
            _ => "make the transport carry stdout unchanged and write diagnostics to stderr",
        }
    }
}
