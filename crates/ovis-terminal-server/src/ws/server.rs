//! Serving a WebSocket endpoint: bounded accepts, checked upgrades and the shared
//! connection core.
//!
//! Every connection task lives in a `JoinSet` owned by the accept loop, so shutdown
//! closes, joins and, past the grace period, aborts all of them; nothing is detached.

use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{Authority, EndpointPolicy, PeerEvidence, RemoteTransport};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_protocol::wire::{FRAME_HEADER_LEN, MAX_FRAME_PAYLOAD};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::frame::FrameDecoder;
use super::handle::{WsReport, WsServerHandle};
use super::{AllowedOrigins, WsEndpoint, WsError, pump, upgrade};
use crate::conn::{Connections, RequestHandler};

const DEFAULT_MAX_CONNECTIONS: usize = 64;
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// How long the close frame may take to leave once the connection itself has ended.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// The largest message: one maximal frame, header included. A record never needs more.
const MAX_MESSAGE: usize = MAX_FRAME_PAYLOAD + FRAME_HEADER_LEN;

/// What a WebSocket server authenticates against, which origins it admits and how far
/// it scales.
#[derive(Debug, Clone)]
pub struct WsConfig {
    authority: Arc<Authority<RegistryIdentities>>,
    runtime: RuntimeIdentity,
    policy: EndpointPolicy,
    origins: AllowedOrigins,
    max_connections: usize,
    handshake_timeout: Duration,
    shutdown_grace: Duration,
}

impl WsConfig {
    /// A server of `runtime` admitting peers under `policy` and authenticating them against
    /// `authority`; no browser origin is admitted and 64 connections are served at most.
    #[must_use]
    pub fn new(
        authority: Arc<Authority<RegistryIdentities>>,
        runtime: RuntimeIdentity,
        policy: EndpointPolicy,
    ) -> Self {
        Self {
            authority,
            runtime,
            policy,
            origins: AllowedOrigins::default(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
        }
    }

    /// Admits upgrades from exactly these browser origins.
    #[must_use]
    pub fn with_origins(mut self, origins: AllowedOrigins) -> Self {
        self.origins = origins;
        self
    }

    /// Caps concurrent connections; further peers wait in the listen backlog.
    #[must_use]
    pub const fn with_max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// Bounds how long a peer may take to send its upgrade request.
    #[must_use]
    pub const fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }
}

/// What every connection task of one server shares.
struct Shared<H> {
    connections: Connections<H>,
    origins: AllowedOrigins,
    policy: EndpointPolicy,
    handshake_timeout: Duration,
}

struct Serving<H> {
    shared: Arc<Shared<H>>,
    shutdown: watch::Receiver<bool>,
    progress: watch::Sender<WsReport>,
    max_connections: usize,
    grace: Duration,
}

/// Serves `endpoint` with `handler` until [`WsServerHandle::shutdown`].
///
/// # Errors
///
/// [`WsError::Endpoint`] when the hello result cannot be formed and [`WsError::Bind`]
/// when the endpoint cannot report its address.
pub fn serve_websocket<H: RequestHandler>(
    endpoint: WsEndpoint,
    config: WsConfig,
    handler: Arc<H>,
) -> Result<WsServerHandle, WsError> {
    let address = endpoint.local_addr()?;
    let connections =
        Connections::new(config.authority, config.runtime, handler).map_err(WsError::Endpoint)?;
    let (signal, shutdown) = watch::channel(false);
    let (progress, observed) = watch::channel(WsReport::default());
    let serving = Serving {
        shared: Arc::new(Shared {
            connections,
            origins: config.origins,
            policy: config.policy,
            handshake_timeout: config.handshake_timeout,
        }),
        shutdown,
        progress,
        max_connections: config.max_connections,
        grace: config.shutdown_grace,
    };
    Ok(WsServerHandle {
        address,
        signal,
        progress: observed,
        task: tokio::spawn(accept_loop(endpoint, serving)),
    })
}

async fn accept_loop<H: RequestHandler>(
    endpoint: WsEndpoint,
    mut serving: Serving<H>,
) -> Result<WsReport, WsError> {
    let mut report = WsReport::default();
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
                match accepted {
                    Ok(stream) => {
                        report.accept();
                        running.spawn(serve_one(stream, Arc::clone(&serving.shared)));
                    }
                    Err(error) => break Err(error),
                }
            }
        }
    };
    serving.shared.connections.shutdown();
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
    stopped.map(|()| report)
}

/// Upgrades one TCP connection, then serves it with the shared connection core.
async fn serve_one<H: RequestHandler>(
    mut stream: TcpStream,
    shared: Arc<Shared<H>>,
) -> Result<(), WsError> {
    let answered = upgrade::answer(&mut stream, &shared.origins);
    let Ok(read_ahead) = tokio::time::timeout(shared.handshake_timeout, answered).await else {
        return Err(WsError::HandshakeTimeout {
            after_ms: shared.handshake_timeout.as_millis(),
        });
    };
    let mut decoder = FrameDecoder::new(MAX_MESSAGE);
    decoder.feed(&read_ahead?);
    let admission = shared
        .policy
        .admit(&PeerEvidence::Remote(RemoteTransport::WebSocket));
    let (bytes, pump) = pump::channel(stream, decoder);
    let serving = shared.connections.serve(bytes, admission);
    let pumping = pump.run();
    tokio::pin!(serving, pumping);
    let (served, pumped) = tokio::select! {
        served = &mut serving => {
            // A peer that stopped reading may never take the close frame; the connection
            // already ended with its own cause, so a lapsed grace adds no failure.
            let closed = tokio::time::timeout(CLOSE_GRACE, &mut pumping).await;
            (served, closed.unwrap_or(Ok(())))
        }
        pumped = &mut pumping => ((&mut serving).await, pumped),
    };
    served.map_err(WsError::Connection)?;
    pumped
}
