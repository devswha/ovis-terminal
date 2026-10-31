//! Serving a Unix endpoint: bounded accepts and joined, cancellation-scoped connections.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthzError};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use tokio::sync::watch;
use tokio::task::{JoinError, JoinHandle, JoinSet};

use super::{EndpointError, UnixEndpoint};
use crate::conn::{ConnectionError, Connections, RequestHandler};

const DEFAULT_MAX_CONNECTIONS: usize = 64;
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// What a server authenticates against, reports as its identity and how far it scales.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    authority: Arc<Authority<RegistryIdentities>>,
    runtime: RuntimeIdentity,
    max_connections: usize,
    shutdown_grace: Duration,
}

impl ServerConfig {
    /// A server of `runtime` authenticating against `authority`, with 64 connections at most.
    #[must_use]
    pub const fn new(
        authority: Arc<Authority<RegistryIdentities>>,
        runtime: RuntimeIdentity,
    ) -> Self {
        Self {
            authority,
            runtime,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
        }
    }

    /// Caps concurrent connections; further peers wait in the listen backlog.
    #[must_use]
    pub const fn with_max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }
}

/// How the connections of one server ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeReport {
    accepted: u64,
    failures: BTreeMap<&'static str, u64>,
    panicked: u64,
    aborted: u64,
}

impl ServeReport {
    /// Connections accepted.
    #[must_use]
    pub const fn accepted(&self) -> u64 {
        self.accepted
    }

    /// Connections the server refused or closed for a failure, by failure kind.
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

    fn count(&mut self, kind: &'static str) {
        let count = self.failures.entry(kind).or_default();
        *count = count.saturating_add(1);
    }

    fn record(&mut self, ended: Result<Result<(), ConnectionError>, JoinError>) {
        match ended {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.count(error.kind()),
            Err(error) if error.is_panic() => self.panicked = self.panicked.saturating_add(1),
            Err(_) => self.aborted = self.aborted.saturating_add(1),
        }
    }
}

/// A running server; dropping it cancels the server and every connection.
#[derive(Debug)]
pub struct ServerHandle {
    signal: watch::Sender<bool>,
    progress: watch::Receiver<ServeReport>,
    task: JoinHandle<Result<ServeReport, EndpointError>>,
}

impl ServerHandle {
    /// How the connections that already ended ended, updated as each one is joined.
    #[must_use]
    pub fn progress(&self) -> watch::Receiver<ServeReport> {
        self.progress.clone()
    }

    /// Stops accepting, closes every connection with `$/closing` and joins them.
    ///
    /// # Errors
    ///
    /// The accept failure that stopped the server early, [`EndpointError::Io`] when the
    /// socket file cannot be removed and [`EndpointError::ServerTask`] if the task panicked.
    pub async fn shutdown(mut self) -> Result<ServeReport, EndpointError> {
        self.signal.send_replace(true);
        (&mut self.task).await.map_err(EndpointError::ServerTask)?
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Serving<H> {
    connections: Connections<H>,
    shutdown: watch::Receiver<bool>,
    progress: watch::Sender<ServeReport>,
    max_connections: usize,
    grace: Duration,
}

/// Serves `endpoint` with `handler` until [`ServerHandle::shutdown`].
///
/// # Errors
///
/// The errors of [`Connections::new`] when the hello result cannot be formed.
pub fn serve_unix<H: RequestHandler>(
    endpoint: UnixEndpoint,
    config: ServerConfig,
    handler: Arc<H>,
) -> Result<ServerHandle, EndpointError> {
    let connections = Connections::new(config.authority, config.runtime, handler)?;
    let (signal, shutdown) = watch::channel(false);
    let (progress, observed) = watch::channel(ServeReport::default());
    let serving = Serving {
        connections,
        shutdown,
        progress,
        max_connections: config.max_connections,
        grace: config.shutdown_grace,
    };
    let task = tokio::spawn(accept_loop(endpoint, serving));
    Ok(ServerHandle {
        signal,
        progress: observed,
        task,
    })
}

async fn accept_loop<H: RequestHandler>(
    endpoint: UnixEndpoint,
    mut serving: Serving<H>,
) -> Result<ServeReport, EndpointError> {
    let mut report = ServeReport::default();
    let mut running = JoinSet::new();
    let stopped = loop {
        tokio::select! {
            biased;
            _ = serving.shutdown.changed() => break Ok(()),
            Some(ended) = running.join_next(), if !running.is_empty() => {
                report.record(ended);
                serving.progress.send_replace(report.clone());
            }
            accepted = endpoint.accept(), if running.len() < serving.max_connections => {
                let (stream, evidence) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => break Err(error),
                };
                report.accepted = report.accepted.saturating_add(1);
                // A peer the operating system cannot describe is refused, never trusted.
                let admission = evidence
                    .inspect_err(|error| report.count(error.kind()))
                    .map_err(|_| AuthzError::PeerRejected)
                    .and_then(|evidence| endpoint.policy().admit(&evidence));
                running.spawn(serving.connections.serve(stream, admission));
            }
        }
    };
    serving.connections.shutdown();
    let drained = tokio::time::timeout(serving.grace, async {
        while let Some(ended) = running.join_next().await {
            report.record(ended);
        }
    })
    .await;
    if drained.is_err() {
        running.abort_all();
        while let Some(ended) = running.join_next().await {
            report.record(ended);
        }
    }
    serving.progress.send_replace(report.clone());
    let removed = endpoint.remove();
    stopped?;
    removed?;
    Ok(report)
}
