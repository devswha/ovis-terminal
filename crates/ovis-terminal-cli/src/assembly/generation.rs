//! What one boot identity serves: the dispatcher of the hosted lanes, the handler in front of
//! it and both listeners. A planned restart closes every attach stream `server_restart`,
//! ends the old listeners' connections with `$/closing{server_restart}`, joins the old
//! dispatcher and serves a new generation under the successor identity on the same
//! endpoint, then says so on stdout.

use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_authz::Authority;
use ovis_terminal_core::health::Health;
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_core::restart::Control;
use ovis_terminal_core::telemetry::redaction::RetentionPolicy;
use ovis_terminal_protocol::schema::limits::TransportKind;
use ovis_terminal_server::authn::Revalidated;
use ovis_terminal_server::dispatch::lanes::boundary_policy::{AuditTrail, BoundaryPolicy};
use ovis_terminal_server::dispatch::lanes::health::Measured;
use ovis_terminal_server::dispatch::remote::RemoteDispatch;
use ovis_terminal_server::listen::{
    ServeReport, ServerConfig, ServerHandle, UnixEndpoint, serve_unix,
};
use ovis_terminal_server::streams::{AttachHandler, AttachLane};
use ovis_terminal_server::ws::WsServerHandle;
use ovis_terminal_store::OpId as StoreOpId;

use super::AssemblyError;
use super::actor::Actor;
use super::directory::Directory;
use super::handler::{Assembled, Handler, Served};
use super::journal::BootJournal;
use super::lanes::agents::Agents;
use super::lanes::{Dispatch, DispatchLog, Hosting, host};
use super::restart::{RestartSignal, StopSignal};

/// Everything one server lifetime keeps across planned restarts.
#[derive(Debug)]
pub(super) struct Shared {
    pub(super) state: PathBuf,
    pub(super) socket: PathBuf,
    pub(super) websocket_port: Option<u16>,
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) directory: Arc<Directory>,
    pub(super) lane: Arc<AttachLane<Arc<Directory>>>,
    pub(super) actor: Actor,
    pub(super) control: Arc<Control>,
    pub(super) agents: Arc<Agents>,
    pub(super) journal: Arc<BootJournal>,
    pub(super) stop: StopSignal,
    pub(super) restart: RestartSignal,
}

/// Calls each boot's audit trail keeps, and for how long.
const AUDIT_RECORDS: usize = 1024;
const AUDIT_RETAINED_MS: u64 = 10 * 60 * 1000;

/// The audit trail of `identity`'s boot.
fn audit(identity: RuntimeIdentity) -> Result<Arc<AuditTrail>, AssemblyError> {
    let records = NonZeroUsize::new(AUDIT_RECORDS)
        .ok_or_else(|| AssemblyError::start("size the audit trail")("no records"))?;
    let policy = RetentionPolicy::new(records, AUDIT_RETAINED_MS);
    Ok(Arc::new(AuditTrail::new(identity, policy)))
}

/// What ending a generation leaves: its listener's report, its dispatch log, and after a
/// planned restart the endpoint, still bound, for the successor.
pub(super) struct Ended {
    pub(super) report: ServeReport,
    pub(super) log: Arc<DispatchLog>,
    pub(super) endpoint: Option<UnixEndpoint>,
}

/// Bound on handing every stream's restart close to its connection; a client that never
/// reads cannot hold a planned restart longer than the listener's own shutdown grace.
const DELIVERY_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Waits until every stream closed `server_restart` has queued its close on its
/// connection: each delivery task ends once it did, so every close precedes the
/// connection's `$/closing`. Past the bound the restart proceeds and says so.
async fn delivered(lane: &AttachLane<Arc<Directory>>) -> Result<(), AssemblyError> {
    let Ok(joined) = tokio::time::timeout(DELIVERY_GRACE, lane.shutdown()).await else {
        let warning = serde_json::json!({"warning": {
            "stage": "deliver the restart closes",
            "detail": "a stream did not take its close within the grace",
        }});
        let mut stderr = std::io::stderr().lock();
        match writeln!(stderr, "{warning}") {
            Ok(()) | Err(_) => return Ok(()),
        }
    };
    joined.map_err(AssemblyError::stop("deliver the restart closes"))
}

/// How a generation's listeners end.
#[derive(Debug, Clone, Copy)]
pub(super) enum Ending {
    /// The server stops: connections close `server_shutdown`.
    Shutdown,
    /// A successor boot serves next: connections close `server_restart`.
    Restart,
}

/// The listeners, handler and dispatcher of one boot identity.
pub(super) struct Generation {
    pub(super) identity: RuntimeIdentity,
    pub(super) websocket: Option<WsServerHandle>,
    listener: ServerHandle,
    dispatch: Arc<Dispatch>,
    log: Arc<DispatchLog>,
    events: Arc<super::events::EventLane>,
    handler: Arc<Handler>,
}

impl Generation {
    /// Hosts the lanes for `identity`, measured into `health`, and serves them on the
    /// endpoint and, when the lifetime has one, the WebSocket port.
    pub(super) async fn open(
        shared: &Shared,
        (identity, health, endpoint): (RuntimeIdentity, Arc<Health>, Option<UnixEndpoint>),
    ) -> Result<Self, AssemblyError> {
        let (dispatch, log, routes) = host(&Hosting {
            state: &shared.state,
            identity,
            authority: Arc::clone(&shared.authority),
            directory: Arc::clone(&shared.directory),
            actor: shared.actor.clone(),
            control: Arc::clone(&shared.control),
            agents: Arc::clone(&shared.agents),
            health: Arc::clone(&health),
        })?;
        let feed = super::feed::subscribe(&log)?;
        shared
            .actor
            .call(move |runtime| {
                runtime.status_feed = Some(feed);
                Ok(())
            })
            .await
            .map_err(AssemblyError::start("install the status feed"))?;
        let events = super::events::EventLane::start(&log, identity, Arc::clone(&health))?;
        let dispatch = Arc::new(dispatch);
        let served = Arc::new(Served {
            actor: shared.actor.clone(),
            lane: Arc::clone(&shared.lane),
            dispatch: Arc::clone(&dispatch),
            events: Arc::clone(&events),
            stop: shared.stop.clone(),
            restart: shared.restart.clone(),
        });
        let transports = match shared.websocket_port {
            Some(_) => vec![TransportKind::Unix, TransportKind::Websocket],
            None => vec![TransportKind::Unix],
        };
        let assembled = Assembled {
            attach: AttachHandler::new(Arc::clone(&served), Arc::clone(&shared.lane)),
            served,
            transports,
        };
        let remote = RemoteDispatch::new(routes, Arc::new(assembled));
        let gated = shared.agents.gate(Arc::new(remote));
        let policed = BoundaryPolicy::new((Arc::clone(&shared.authority), audit(identity)?), gated);
        let measured = Measured::new(health, Arc::new(policed));
        let handler = Arc::new(Revalidated::new(Arc::clone(&shared.authority), measured));
        let endpoint = match endpoint {
            Some(endpoint) => endpoint,
            None => UnixEndpoint::bind(&shared.socket)
                .await
                .map_err(AssemblyError::Endpoint)?,
        };
        let config = ServerConfig::new(Arc::clone(&shared.authority), identity);
        let listener =
            serve_unix(endpoint, config, Arc::clone(&handler)).map_err(AssemblyError::Endpoint)?;
        let websocket = match shared.websocket_port {
            Some(port) => {
                let binding = (Arc::clone(&shared.authority), identity);
                Some(super::websocket::start(port, binding, &handler).await?)
            }
            None => None,
        };
        Ok(Self {
            identity,
            websocket,
            listener,
            dispatch,
            log,
            events,
            handler,
        })
    }

    /// The WebSocket port every later generation keeps listening on.
    pub(super) fn websocket_port(&self) -> Option<u16> {
        self.websocket
            .as_ref()
            .map(|listener| listener.local_addr().port())
    }

    /// Ends both listeners as `ending` says and joins the dispatcher; its log is returned
    /// for the caller to close once nothing else holds it, and after a planned restart the
    /// still-bound endpoint for the successor.
    pub(super) async fn end(self, ending: Ending) -> Result<Ended, AssemblyError> {
        if let Some(websocket) = self.websocket {
            websocket
                .shutdown()
                .await
                .map_err(AssemblyError::stop("close the WebSocket listener"))?;
        }
        let ended = match ending {
            Ending::Shutdown => self.listener.shutdown().await.map(|report| (report, None)),
            Ending::Restart => self.listener.hand_over().await,
        };
        let (report, endpoint) = ended.map_err(AssemblyError::stop("close the endpoint"))?;
        self.dispatch
            .shutdown()
            .await
            .map_err(AssemblyError::stop("join dispatched operations"))?;
        drop((self.dispatch, self.handler));
        self.events.shutdown()?;
        Ok(Ended {
            report,
            log: self.log,
            endpoint,
        })
    }

    /// Follows a call that may have restarted control under `op`. When control serves a
    /// successor boot, every stream, listener and log moves to it and the old listener's
    /// report is returned; otherwise this generation keeps serving.
    pub(super) async fn hand_over(
        self,
        shared: &Shared,
        op: String,
    ) -> Result<(Self, Option<ServeReport>), AssemblyError> {
        let successor = shared
            .control
            .identity()
            .map_err(AssemblyError::stop("read the successor boot"))?;
        if successor == self.identity {
            return Ok((self, None));
        }
        let op = StoreOpId::new(op);
        shared
            .actor
            .call(move |runtime| runtime.restarting(&op))
            .await
            .map_err(AssemblyError::stop("close attach streams for the restart"))?;
        delivered(&shared.lane).await?;
        let Ended {
            report,
            log,
            endpoint,
        } = self.end(Ending::Restart).await?;
        super::lifetime::close(log, "close the dispatch log of the old boot").await?;
        let journal = Arc::clone(&shared.journal);
        tokio::task::spawn_blocking(move || journal.reopen(successor))
            .await
            .map_err(AssemblyError::stop("reopen the lease log"))??;
        let health = super::health::open(&shared.state, successor, None)?;
        let (state, measured) = (shared.state.clone(), Arc::clone(&health));
        shared
            .actor
            .call(move |runtime| runtime.recovered((successor, &state, measured)))
            .await
            .map_err(AssemblyError::start("serve the successor boot"))?;
        let next = Self::open(shared, (successor, health, endpoint)).await?;
        Ok((next, Some(report)))
    }
}
