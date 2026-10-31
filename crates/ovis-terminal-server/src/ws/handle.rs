//! A running WebSocket server: its progress report and its shutdown.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use tokio::sync::watch;
use tokio::task::{JoinError, JoinHandle};

use super::WsError;

/// How the connections of one WebSocket server ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WsReport {
    accepted: u64,
    failures: BTreeMap<&'static str, u64>,
    panicked: u64,
    aborted: u64,
}

impl WsReport {
    /// TCP connections accepted, upgraded or not.
    #[must_use]
    pub const fn accepted(&self) -> u64 {
        self.accepted
    }

    /// Connections refused or closed for a failure, by [`WsError::kind`].
    #[must_use]
    pub const fn failures(&self) -> &BTreeMap<&'static str, u64> {
        &self.failures
    }

    /// Connection tasks that panicked.
    #[must_use]
    pub const fn panicked(&self) -> u64 {
        self.panicked
    }

    /// Connections cancelled because they outlived the shutdown grace period.
    #[must_use]
    pub const fn aborted(&self) -> u64 {
        self.aborted
    }

    pub(super) const fn accept(&mut self) {
        self.accepted = self.accepted.saturating_add(1);
    }

    pub(super) fn record(&mut self, ended: Result<Result<(), WsError>, JoinError>) {
        match ended {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let count = self.failures.entry(error.kind()).or_default();
                *count = count.saturating_add(1);
            }
            Err(error) if error.is_panic() => self.panicked = self.panicked.saturating_add(1),
            Err(_) => self.aborted = self.aborted.saturating_add(1),
        }
    }
}

/// A running WebSocket server; dropping it cancels the server and every connection.
#[derive(Debug)]
pub struct WsServerHandle {
    pub(super) address: SocketAddr,
    pub(super) signal: watch::Sender<bool>,
    pub(super) progress: watch::Receiver<WsReport>,
    pub(super) task: JoinHandle<Result<WsReport, WsError>>,
}

impl WsServerHandle {
    /// The address the server listens on.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// How the connections that already ended ended, updated as each one is joined.
    #[must_use]
    pub fn progress(&self) -> watch::Receiver<WsReport> {
        self.progress.clone()
    }

    /// Stops accepting, closes every connection with `$/closing` and joins them.
    ///
    /// # Errors
    ///
    /// The accept failure that stopped the server early, or [`WsError::ServerTask`] when
    /// the server task panicked.
    pub async fn shutdown(mut self) -> Result<WsReport, WsError> {
        self.signal.send_replace(true);
        (&mut self.task).await.map_err(WsError::ServerTask)?
    }
}

impl Drop for WsServerHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}
