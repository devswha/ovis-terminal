//! Local endpoint admission. Admission proves which account connected over a private
//! endpoint; it grants nothing, so every admitted connection still authenticates.

mod pipe;

pub use pipe::{
    Ace, AceKind, Dacl, PipeInstance, PipeInstanceEvidence, PipeNamespace, PipeSecurity,
    RemoteClients,
};

use crate::error::{AuthzError, EndpointDefect};

/// Mode bits granting the group or other users any access.
const SHARED_MODE_BITS: u32 = 0o077;

/// A Unix user id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnixUid(u32);

impl UnixUid {
    /// Wraps a native user id.
    #[must_use]
    pub const fn new(uid: u32) -> Self {
        Self(uid)
    }
}

/// A Windows security identifier such as `S-1-5-21-...`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowsSid(String);

impl WindowsSid {
    /// Parses the `S-1-<authority>[-<sub-authority>...]` text form.
    ///
    /// # Errors
    ///
    /// Returns [`AuthzError::InvalidValue`] for any other text.
    pub fn parse(text: &str) -> Result<Self, AuthzError> {
        let parts = text.strip_prefix("S-1-").map(|rest| rest.split('-'));
        let numeric = parts.is_some_and(|mut parts| {
            parts.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        });
        if !numeric {
            return Err(AuthzError::InvalidValue { field_path: "sid" });
        }
        Ok(Self(text.to_owned()))
    }
}

/// The account that owns the runtime and its endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalAccount {
    /// A Unix user.
    Unix(UnixUid),
    /// A Windows account.
    Windows(WindowsSid),
}

/// Owner and permission bits of a socket file or its directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileEvidence {
    owner: UnixUid,
    mode: u32,
}

impl FileEvidence {
    /// A file owned by `owner` with permission bits `mode`.
    #[must_use]
    pub const fn new(owner: UnixUid, mode: u32) -> Self {
        Self { owner, mode }
    }

    fn defect(self, owner: UnixUid) -> Option<EndpointDefect> {
        if self.owner != owner {
            Some(EndpointDefect::ForeignOwner)
        } else if self.mode & SHARED_MODE_BITS != 0 {
            Some(EndpointDefect::SharedMode)
        } else {
            None
        }
    }
}

/// What the operating system reports about a Unix-socket connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnixSocketEvidence {
    socket: FileEvidence,
    directory: FileEvidence,
    peer: UnixUid,
}

impl UnixSocketEvidence {
    /// The socket file, its directory and the peer's credential (`SO_PEERCRED`).
    #[must_use]
    pub const fn new(socket: FileEvidence, directory: FileEvidence, peer: UnixUid) -> Self {
        Self {
            socket,
            directory,
            peer,
        }
    }
}

/// A transport whose peer is authenticated only by credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoteTransport {
    /// The authenticated WebSocket bridge.
    WebSocket,
    /// A tunnel over SSH.
    Ssh,
}

/// What is known about a connecting peer before it authenticates.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PeerEvidence {
    /// A local Unix-socket connection.
    UnixSocket(UnixSocketEvidence),
    /// A connection accepted on one named-pipe instance.
    NamedPipe {
        /// The instance that accepted the connection.
        instance: PipeInstanceEvidence,
        /// The connecting client's account.
        client: WindowsSid,
    },
    /// A remote connection.
    Remote(RemoteTransport),
}

/// The transport of an admitted connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// A local Unix socket.
    UnixSocket,
    /// A local named pipe.
    NamedPipe,
    /// A WebSocket.
    WebSocket,
    /// An SSH tunnel.
    Ssh,
}

impl Transport {
    /// The stable token of the transport for logs and audit records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnixSocket => "unix_socket",
            Self::NamedPipe => "named_pipe",
            Self::WebSocket => "websocket",
            Self::Ssh => "ssh",
        }
    }
}

/// Proof that a connection passed endpoint admission; required to authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAdmission {
    transport: Transport,
}

impl PeerAdmission {
    pub(super) const fn transport(self) -> Transport {
        self.transport
    }
}

/// Checks local endpoints against the account that owns the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointPolicy {
    owner: LocalAccount,
}

impl EndpointPolicy {
    /// A policy for endpoints owned by `owner`.
    #[must_use]
    pub const fn new(owner: LocalAccount) -> Self {
        Self { owner }
    }

    /// Admits a connection whose endpoint is private to the owner and whose local peer is
    /// the owner; remote peers are admitted to authenticate with credential material.
    ///
    /// # Errors
    ///
    /// [`AuthzError::EndpointInsecure`] when the endpoint is reachable by other accounts and
    /// [`AuthzError::PeerRejected`] when the local peer is another account.
    pub fn admit(&self, evidence: &PeerEvidence) -> Result<PeerAdmission, AuthzError> {
        let (transport, peer_is_owner) = match (evidence, &self.owner) {
            (PeerEvidence::UnixSocket(socket), LocalAccount::Unix(owner)) => {
                let defect = socket
                    .socket
                    .defect(*owner)
                    .or_else(|| socket.directory.defect(*owner));
                if let Some(defect) = defect {
                    return Err(AuthzError::EndpointInsecure { defect });
                }
                (Transport::UnixSocket, socket.peer == *owner)
            }
            (PeerEvidence::NamedPipe { instance, client }, LocalAccount::Windows(owner)) => {
                pipe::check_security(instance, owner)?;
                (Transport::NamedPipe, client == owner)
            }
            (PeerEvidence::Remote(RemoteTransport::WebSocket), _) => (Transport::WebSocket, true),
            (PeerEvidence::Remote(RemoteTransport::Ssh), _) => (Transport::Ssh, true),
            (PeerEvidence::UnixSocket(_), LocalAccount::Windows(_))
            | (PeerEvidence::NamedPipe { .. }, LocalAccount::Unix(_)) => {
                return Err(AuthzError::EndpointInsecure {
                    defect: EndpointDefect::ForeignOwner,
                });
            }
        };
        if !peer_is_owner {
            return Err(AuthzError::PeerRejected);
        }
        Ok(PeerAdmission { transport })
    }
}
