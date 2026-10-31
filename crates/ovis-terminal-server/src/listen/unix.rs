//! A private Unix-socket endpoint: safe binding, stale-path takeover and peer evidence.

use std::fs::{self, DirBuilder, Metadata};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use ovis_terminal_authz::{
    EndpointPolicy, FileEvidence, LocalAccount, PeerEvidence, UnixSocketEvidence, UnixUid,
};
use tokio::net::{UnixListener, UnixStream};

use super::EndpointError;

const PERMISSION_BITS: u32 = 0o777;

/// What occupies a socket path; only connecting decides whether a socket is live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathOccupant {
    Vacant,
    Socket,
    Other,
}

pub(super) fn occupant(path: &Path) -> Result<PathOccupant, EndpointError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => Ok(PathOccupant::Socket),
        Ok(_) => Ok(PathOccupant::Other),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(PathOccupant::Vacant),
        Err(source) => Err(io_error("inspect", path, source)),
    }
}

/// A bound socket whose directory and file are private to the account that bound it.
#[derive(Debug)]
pub struct UnixEndpoint {
    listener: UnixListener,
    path: PathBuf,
    directory: PathBuf,
    inode: (u64, u64),
    policy: EndpointPolicy,
}

impl UnixEndpoint {
    /// Binds `path`, replacing only a stale socket nobody listens on.
    ///
    /// The directory is created with mode 0700; the socket gets mode 0600. A live socket,
    /// a non-socket file or a directory reachable by another account is refused.
    ///
    /// # Errors
    ///
    /// [`EndpointError::AlreadyServing`], [`EndpointError::NotASocket`],
    /// [`EndpointError::Insecure`] or [`EndpointError::Io`].
    pub async fn bind(path: &Path) -> Result<Self, EndpointError> {
        let directory = path
            .parent()
            .ok_or(EndpointError::InvalidOverride)?
            .to_path_buf();
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(|source| io_error("create directory", &directory, source))?;
        match occupant(path)? {
            PathOccupant::Vacant => {}
            PathOccupant::Other => {
                return Err(EndpointError::NotASocket {
                    path: path.to_path_buf(),
                });
            }
            PathOccupant::Socket => reclaim_stale(path).await?,
        }
        let listener = UnixListener::bind(path).map_err(|source| io_error("bind", path, source))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|source| io_error("restrict", path, source))?;
        let socket = metadata(path)?;
        let owner = UnixUid::new(socket.uid());
        let endpoint = Self {
            listener,
            path: path.to_path_buf(),
            directory,
            inode: (socket.dev(), socket.ino()),
            policy: EndpointPolicy::new(LocalAccount::Unix(owner)),
        };
        let own = PeerEvidence::UnixSocket(endpoint.evidence(owner)?);
        if let Err(source) = endpoint.policy.admit(&own) {
            let path = endpoint.directory.clone();
            endpoint.remove()?;
            return Err(EndpointError::Insecure { path, source });
        }
        Ok(endpoint)
    }

    /// Removes the socket file if it is still the one this endpoint bound.
    ///
    /// # Errors
    ///
    /// [`EndpointError::Io`] when the file cannot be removed.
    pub fn remove(self) -> Result<(), EndpointError> {
        let ours =
            fs::symlink_metadata(&self.path).is_ok_and(|now| (now.dev(), now.ino()) == self.inode);
        if ours {
            fs::remove_file(&self.path)
                .map_err(|source| io_error("remove socket", &self.path, source))?;
        }
        Ok(())
    }

    /// The bound socket path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The policy admitting peers: the account that bound the socket.
    #[must_use]
    pub const fn policy(&self) -> &EndpointPolicy {
        &self.policy
    }

    /// Accepts one connection with the operating system's evidence about its peer.
    ///
    /// # Cancel Safety
    ///
    /// Cancel safe: a dropped call accepts nothing.
    ///
    /// # Errors
    ///
    /// [`EndpointError::Io`] when accepting or inspecting the peer fails.
    pub async fn accept(&self) -> Result<(UnixStream, PeerEvidence), EndpointError> {
        let (stream, _) = self
            .listener
            .accept()
            .await
            .map_err(|source| io_error("accept", &self.path, source))?;
        let peer = stream
            .peer_cred()
            .map_err(|source| io_error("read peer credential", &self.path, source))?;
        let evidence = self.evidence(UnixUid::new(peer.uid()))?;
        Ok((stream, PeerEvidence::UnixSocket(evidence)))
    }

    fn evidence(&self, peer: UnixUid) -> Result<UnixSocketEvidence, EndpointError> {
        let socket = file_evidence(&metadata(&self.path)?);
        let directory = file_evidence(&metadata(&self.directory)?);
        Ok(UnixSocketEvidence::new(socket, directory, peer))
    }
}

async fn reclaim_stale(path: &Path) -> Result<(), EndpointError> {
    match UnixStream::connect(path).await {
        Ok(_) => Err(EndpointError::AlreadyServing {
            path: path.to_path_buf(),
        }),
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
            fs::remove_file(path).map_err(|source| io_error("remove stale socket", path, source))
        }
        Err(source) => Err(io_error("connect", path, source)),
    }
}

fn metadata(path: &Path) -> Result<Metadata, EndpointError> {
    fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))
}

fn file_evidence(metadata: &Metadata) -> FileEvidence {
    FileEvidence::new(
        UnixUid::new(metadata.uid()),
        metadata.mode() & PERMISSION_BITS,
    )
}

pub(super) fn io_error(operation: &'static str, path: &Path, source: io::Error) -> EndpointError {
    EndpointError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}
