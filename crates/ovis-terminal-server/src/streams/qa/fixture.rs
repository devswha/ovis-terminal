//! A real authority, SQLite coordinator, lease table, screen and attach lane served on a
//! private Unix socket; the test feeds holder output where a holder would.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ovis_terminal_authz::{
    EndpointPolicy, FileEvidence, Grant, Issuance, IssueRequest, LocalAccount, PeerEvidence,
    Permission, Presentation, PrincipalId, ResolvedTarget, ResourceScope, ScopeResource,
    TargetResource, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::events::{Coordinator, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::leases::{LeaseEvent, TerminalLeases, Transition};
use ovis_terminal_core::registry::{TargetResolutionError, TerminalGeneration, TerminalId};
use ovis_terminal_core::screen::{ReplyBatch, Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_holder::protocol::{
    Exit, Output, ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};
use ovis_terminal_server::listen::ServerHandle;
use ovis_terminal_server::streams::{
    AttachHandler, AttachLane, LeaseJournal, SessionBacklog, StreamError, StreamLimits,
    StreamSetup, TerminalStreams,
};
use ovis_terminal_store::{OpId, Snapshot, Store, StoreOptions};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};
use serde_json::Value;
use tokio::sync::watch;

use super::TestResult;
use super::socket::{self, Client, runtime};

/// The terminal every scenario attaches to.
pub(crate) const TERMINAL: &str = "terminal:7";
const WAIT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(crate) struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _snapshot: &Snapshot, _filter: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

type Core = Coordinator<LeaseEvent, NoSections>;

/// Commits through the real coordinator, then publishes what it committed.
#[derive(Debug)]
pub(crate) struct Recorder {
    core: Arc<Core>,
    committed: watch::Sender<Vec<LeaseEvent>>,
}

impl LeaseJournal for Recorder {
    fn record(&self, op: OpId, transition: Transition) -> Result<(), StreamError> {
        self.core.record(op, transition.clone())?;
        self.committed
            .send_modify(|events| events.extend(transition.events));
        Ok(())
    }
}

impl Recorder {
    /// Waits for a committed revocation of a lease held by client `label`.
    pub(crate) async fn revoked(&self, label: &str) -> TestResult {
        let mut committed = self.committed.subscribe();
        let wait = committed.wait_for(|events| {
            events.iter().any(|event| match event {
                LeaseEvent::Revoked { lease, .. } => lease.owner.client.as_str() == label,
                _ => false,
            })
        });
        tokio::time::timeout(WAIT, wait).await??;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Directory(ResolvedTerminal);

impl TerminalDirectory for Directory {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::TerminalId(id) if id.as_str() == TERMINAL => Ok(self.0),
            _ => Err(TargetResolutionError::NeedsTarget),
        }
    }
}

#[derive(Debug)]
struct Nothing;

impl RequestHandler for Nothing {
    fn methods(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn call(&self, _call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        std::future::ready(Err(BoundaryError::method_not_found()))
    }
}

pub(crate) struct Fixture {
    socket: socket::Fixture,
    server: Option<ServerHandle>,
    pub hub: Arc<TerminalStreams>,
    pub lane: Arc<AttachLane<Directory>>,
    pub journal: Arc<Recorder>,
    pub backlog: Arc<SessionBacklog>,
    core: Arc<Core>,
    bearers: Mutex<HashMap<String, String>>,
    target: TerminalTarget,
    pub raw: Vec<u8>,
}

/// Grid every screen and reference VT uses.
pub(crate) fn vt_config() -> TestResult<VtConfig> {
    Ok(VtConfig::new(Geometry::new(40, 8)?, HistoryLimit::new(200)))
}

impl Fixture {
    pub(crate) async fn new(limits: StreamLimits, session_cap: u64) -> TestResult<Self> {
        let socket = socket::Fixture::new()?;
        let identity = runtime(1)?;
        let store = Store::open(&socket.path("db"), StoreOptions::default())?;
        let core = Arc::new(Coordinator::new(store, identity, NoSections));
        let (committed, _) = watch::channel(Vec::new());
        let journal = Arc::new(Recorder {
            core: Arc::clone(&core),
            committed,
        });
        let terminal = TerminalId::try_from(7)?;
        let generation = TerminalGeneration::new(1)?;
        let resolved = ResolvedTerminal::new(identity.machine_id(), terminal, generation);
        let leases = TerminalLeases::new(
            Arc::clone(&socket.authority),
            ResolvedTarget::new(
                identity.machine_id(),
                TargetResource::Terminal {
                    placement: None,
                    terminal,
                    incarnation: generation,
                },
            ),
        )?;
        let target = TerminalTarget {
            terminal_id: HolderTerminalId::new(TERMINAL.to_owned()),
            terminal_incarnation: TerminalIncarnation::new(1),
            process_incarnation: Some(ProcessIncarnation::new(1)),
        };
        let screen = Screen::new(
            target.clone(),
            ScreenConfig::new(vt_config()?, ScreenBounds::DEFAULT),
        )?;
        let backlog = Arc::new(SessionBacklog::new(session_cap));
        let setup = StreamSetup::new(
            limits,
            Arc::clone(&backlog),
            Arc::<Recorder>::clone(&journal),
        );
        let hub = TerminalStreams::new(screen, leases, setup);
        let lane = AttachLane::new(Directory(resolved));
        lane.install((terminal, generation), Arc::clone(&hub))?;
        let handler = Arc::new(AttachHandler::new(Arc::new(Nothing), Arc::clone(&lane)));
        let server = socket.serve(1, handler).await?;
        Ok(Self {
            socket,
            server: Some(server),
            hub,
            lane,
            journal,
            backlog,
            core,
            bearers: Mutex::new(HashMap::new()),
            target,
            raw: Vec::new(),
        })
    }

    /// Issues `permissions` on the terminal to principal `label`; `owner` gets the
    /// bootstrap owner credential.
    pub(crate) fn issue(&self, label: &str, permissions: &[Permission]) -> TestResult {
        let authority = &self.socket.authority;
        let uid = UnixUid::new(1000);
        let socket = UnixSocketEvidence::new(
            FileEvidence::new(uid, 0o600),
            FileEvidence::new(uid, 0o700),
            uid,
        );
        let admission = EndpointPolicy::new(LocalAccount::Unix(uid))
            .admit(&PeerEvidence::UnixSocket(socket))?;
        let now = TimestampMs::new(1_000);
        let owner =
            authority.authenticate(&admission, Presentation::new(&self.socket.bearer, now))?;
        let scope = ResourceScope::new(
            runtime(1)?.machine_id(),
            ScopeResource::Terminal {
                id: TerminalId::try_from(7)?,
            },
        );
        let grants = permissions
            .iter()
            .map(|permission| Grant::new(*permission, scope.clone()))
            .collect();
        let request = IssueRequest::new(
            PrincipalId::parse(label)?,
            grants,
            TimestampMs::new(u64::MAX),
        );
        let issued = authority.issue(&owner, Issuance::new(request, now))?;
        self.bearers
            .lock()
            .map_err(|_| "bearers poisoned")?
            .insert(label.to_owned(), issued.bearer().to_owned());
        Ok(())
    }

    /// A connection authenticated as `label` (`owner` for the bootstrap owner).
    pub(crate) async fn connect(&self, label: &str) -> TestResult<Client> {
        let bearer = if label == "owner" {
            self.socket.bearer.clone()
        } else {
            self.bearers
                .lock()
                .map_err(|_| "bearers poisoned")?
                .get(label)
                .cloned()
                .ok_or("principal was not issued")?
        };
        let mut client = self.socket.connect().await?;
        client.hello("1", &bearer).await?;
        Ok(client)
    }

    /// Feeds holder output and returns the query replies the core must write.
    pub(crate) fn publish(&mut self, bytes: &[u8]) -> TestResult<ReplyBatch> {
        let output = Output {
            target: self.target.clone(),
            offset: u64::try_from(self.raw.len())?,
            bytes: bytes.to_vec(),
            hangup: None,
        };
        self.raw.extend_from_slice(bytes);
        Ok(self.hub.publish(&output)?)
    }

    /// Records the program's exit with `code` after every published byte.
    pub(crate) fn exit(&self, code: i32) -> TestResult {
        let exit: Exit = serde_json::from_value(serde_json::json!({
            "target": self.target,
            "code": code,
            "signal": null,
            "exit_unknown": null,
            "io_failure": null,
            "final_offset": self.raw.len().to_string(),
        }))?;
        Ok(self.hub.finish(&exit)?)
    }

    pub(crate) async fn close(mut self) -> TestResult {
        if let Some(server) = self.server.take() {
            server.shutdown().await?;
        }
        self.lane.shutdown().await?;
        drop((self.hub, self.lane, self.journal));
        let core = Arc::try_unwrap(self.core).map_err(|_| "coordinator still shared")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        Ok(())
    }
}
