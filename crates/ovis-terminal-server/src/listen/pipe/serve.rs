//! Serving a pipe endpoint: bounded accepts and joined, cancellation-scoped connections.
//!
//! Every accepted client goes through the same [`Connections`] as a Unix-socket client,
//! so hello, framing, roles, limits and `$/closing` are identical on both transports.
//! Only a failure to create the next instance stops the server; one client's failure
//! ends only its connection.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use super::PipeEndpoint;
use crate::conn::{Connections, RequestHandler};
use crate::listen::EndpointError;

const MAX_CONNECTIONS: usize = 64;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// How the connections of one pipe server ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipeServeReport {
    accepted: u64,
    failures: BTreeMap<&'static str, u64>,
}

impl PipeServeReport {
    /// Connections accepted.
    #[must_use]
    pub const fn accepted(&self) -> u64 {
        self.accepted
    }

    /// Connections refused or closed for a failure, by kind.
    #[must_use]
    pub const fn failures(&self) -> &BTreeMap<&'static str, u64> {
        &self.failures
    }

    fn count(&mut self, kind: &'static str) {
        let count = self.failures.entry(kind).or_default();
        *count = count.saturating_add(1);
    }
}

/// A running pipe server; dropping it cancels the server and every connection.
#[derive(Debug)]
pub struct PipeServerHandle {
    signal: watch::Sender<bool>,
    task: JoinHandle<Result<PipeServeReport, EndpointError>>,
}

impl PipeServerHandle {
    /// Stops accepting, closes every connection with `$/closing` and joins them.
    ///
    /// # Errors
    ///
    /// The accept failure that stopped the server early and
    /// [`EndpointError::ServerTask`] if the task panicked.
    pub async fn shutdown(mut self) -> Result<PipeServeReport, EndpointError> {
        self.signal.send_replace(true);
        (&mut self.task).await.map_err(EndpointError::ServerTask)?
    }
}

impl Drop for PipeServerHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serves `endpoint` through `connections` until [`PipeServerHandle::shutdown`].
#[must_use]
pub fn serve_pipe<H: RequestHandler>(
    endpoint: PipeEndpoint,
    connections: Connections<H>,
) -> PipeServerHandle {
    let (signal, shutdown) = watch::channel(false);
    let task = tokio::spawn(accept_loop(endpoint, Arc::new(connections), shutdown));
    PipeServerHandle { signal, task }
}

type Ended = Option<&'static str>;

async fn accept_loop<H: RequestHandler>(
    mut endpoint: PipeEndpoint,
    connections: Arc<Connections<H>>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<PipeServeReport, EndpointError> {
    let mut report = PipeServeReport::default();
    let mut running: JoinSet<Ended> = JoinSet::new();
    let stopped = loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break Ok(()),
            Some(ended) = running.join_next(), if !running.is_empty() => {
                if let Some(kind) = ended.unwrap_or(Some("connection_task_failed")) {
                    report.count(kind);
                }
            }
            accepted = endpoint.accept(), if running.len() < MAX_CONNECTIONS => {
                let connection = match accepted {
                    Ok(connection) => connection,
                    Err(error) => break Err(error),
                };
                report.accepted = report.accepted.saturating_add(1);
                let connections = Arc::clone(&connections);
                running.spawn(async move {
                    let admission = connection.admit().await;
                    let refused = admission.as_ref().err().map(ovis_terminal_authz::AuthzError::kind);
                    let served = connections.serve(connection, admission).await;
                    refused.or_else(|| served.err().map(|error| error.kind()))
                });
            }
        }
    };
    connections.shutdown();
    let drained = tokio::time::timeout(SHUTDOWN_GRACE, async {
        while let Some(ended) = running.join_next().await {
            if let Some(kind) = ended.unwrap_or(Some("connection_task_failed")) {
                report.count(kind);
            }
        }
    })
    .await;
    if drained.is_err() {
        running.abort_all();
        while running.join_next().await.is_some() {}
    }
    stopped.map(|()| report)
}
