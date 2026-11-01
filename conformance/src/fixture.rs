//! The reusable fixture process and the runner's handle on it.
//!
//! The fixture serves a Unix socket, so everything it serves (`documents`, `handler`,
//! `output`, `serve`, `terminals`) is compiled only where its caller is. Elsewhere the
//! fixture mode reports a typed failure to bind a listener.

#[cfg(unix)]
pub mod agents;
#[cfg(unix)]
pub mod boundary;
#[cfg(unix)]
pub mod bridge;
#[cfg(unix)]
pub mod commit_barrier;
#[cfg(unix)]
pub mod documents;
pub mod graphics;
#[cfg(unix)]
pub mod handler;
#[cfg(unix)]
pub mod health;
#[cfg(unix)]
pub mod holders;
pub mod launch;
#[cfg(unix)]
pub mod launches;
#[cfg(unix)]
pub mod machines;
#[cfg(unix)]
pub mod output;
#[cfg(unix)]
pub mod peer;
#[cfg(unix)]
pub mod processes;
#[cfg(unix)]
pub mod prompts;
#[cfg(unix)]
pub mod rotation;
#[cfg(unix)]
pub mod serve;
#[cfg(unix)]
pub mod services;
#[cfg(unix)]
pub mod template_lane;
#[cfg(unix)]
pub mod terminals;
#[cfg(unix)]
pub mod transcripts;
#[cfg(unix)]
pub mod tree;
#[cfg(unix)]
pub mod update;
pub mod wire;
#[cfg(unix)]
pub mod worktrees;

#[cfg(unix)]
pub(crate) use serve::serve;

/// Which runtime a fixture process is: the one the runner drives, or the second private
/// runtime it reaches as machine `peer`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Role {
    /// The runtime under test; `binary` carries its line to the peer.
    Local {
        /// The binary under test.
        binary: Option<std::path::PathBuf>,
    },
    /// The second private runtime.
    Peer,
}

impl Role {
    /// The role the fixture command line asks for.
    #[must_use]
    pub fn of(peer: bool, binary: Option<std::path::PathBuf>) -> Self {
        if peer {
            Self::Peer
        } else {
            Self::Local { binary }
        }
    }
}

/// Without a Unix listener the fixture cannot bind its endpoint; already settled.
#[cfg(not(unix))]
pub(crate) fn serve(
    _: &std::path::Path,
    _: Role,
) -> std::future::Ready<Result<(), crate::error::RunnerError>> {
    std::future::ready(Err(crate::error::RunnerError::Fixture {
        stage: "bind a listener",
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "the fixture serves only a Unix socket on this platform",
        )),
    }))
}
