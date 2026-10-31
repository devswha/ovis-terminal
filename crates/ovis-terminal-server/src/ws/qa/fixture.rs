//! One attach handler served at once on a private Unix socket and a loopback WebSocket.
//!
//! The handler is real (authority, SQLite coordinator, leases, screen and attach lane);
//! the test feeds holder output where a holder would.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, Grant, Issuance, IssueRequest, OwnerSpec,
    Permission, PrincipalId, ResolvedTarget, ResourceScope, RuntimeBinding, ScopeResource,
    TargetResource,
};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId, TerminalGeneration, TerminalId,
};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_holder::protocol::{
    Exit, Output, ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::dispatch::lanes::agent_status::ResolvedTerminal;
use ovis_terminal_server::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};
use ovis_terminal_server::streams::{
    AttachHandler, AttachLane, SessionBacklog, StreamLimits, StreamSetup, TerminalStreams,
};
use ovis_terminal_server::ws::{
    AllowedOrigins, WsBinding, WsConfig, WsEndpoint, WsReport, WsServerHandle, serve_websocket,
};
use ovis_terminal_store::{Store, StoreOptions};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};
use tokio::io::BufReader;
use tokio::net::UnixStream;

use super::TestResult;
use super::client::WsClient;
use super::seams::{Core, Directory, Journal, NoSections, Nothing, local_admission};
use super::socket::Client;

/// The terminal every scenario attaches to.
pub(crate) const TERMINAL: &str = "terminal:7";

/// The browser origin the fixture's WebSocket admits.
pub(crate) const ORIGIN: &str = "https://ovis.example";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    Unix,
    Ws,
}

pub(crate) fn vt_config() -> TestResult<VtConfig> {
    Ok(VtConfig::new(Geometry::new(40, 8)?, HistoryLimit::new(200)))
}

fn runtime() -> TestResult<RuntimeIdentity> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000028".parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(7)?,
        ServerBootId::try_from(1)?,
    ))
}

pub(crate) struct Fixture {
    root: PathBuf,
    authority: Arc<Authority<RegistryIdentities>>,
    policy: EndpointPolicy,
    bearers: Mutex<HashMap<String, String>>,
    unix: Option<ServerHandle>,
    ws: Option<WsServerHandle>,
    pub ws_address: SocketAddr,
    pub hub: Arc<TerminalStreams>,
    lane: Arc<AttachLane<Directory>>,
    core: Arc<Core>,
    target: TerminalTarget,
    pub raw: Vec<u8>,
}

impl Fixture {
    pub(crate) async fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt28-{}", uuid::Uuid::now_v7()));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        let identity = runtime()?;
        let binding = RuntimeBinding::new(identity.machine_id(), identity.runtime_id());
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        let authority = Arc::new(authority);
        let store = Store::open(&root.join("db"), StoreOptions::default())?;
        let core = Arc::new(Coordinator::new(store, identity, NoSections));
        let (terminal, generation) = (TerminalId::try_from(7)?, TerminalGeneration::new(1)?);
        let resource = TargetResource::Terminal {
            placement: None,
            terminal,
            incarnation: generation,
        };
        let leases = TerminalLeases::new(
            Arc::clone(&authority),
            ResolvedTarget::new(identity.machine_id(), resource),
        )?;
        let target = TerminalTarget {
            terminal_id: HolderTerminalId::new(TERMINAL.to_owned()),
            terminal_incarnation: TerminalIncarnation::new(1),
            process_incarnation: Some(ProcessIncarnation::new(1)),
        };
        let config = ScreenConfig::new(vt_config()?, ScreenBounds::DEFAULT);
        let screen = Screen::new(target.clone(), config)?;
        let backlog = Arc::new(SessionBacklog::new(64 * 1024 * 1024));
        let limits = StreamLimits::advertised(&Limits::v1());
        let setup = StreamSetup::new(limits, backlog, Arc::new(Journal(Arc::clone(&core))));
        let hub = TerminalStreams::new(screen, leases, setup);
        let resolved = ResolvedTerminal::new(identity.machine_id(), terminal, generation);
        let lane = AttachLane::new(Directory(resolved));
        lane.install((terminal, generation), Arc::clone(&hub))?;
        let handler = Arc::new(AttachHandler::new(Arc::new(Nothing), Arc::clone(&lane)));
        let endpoint = UnixEndpoint::bind(&root.join("run").join("s.sock")).await?;
        let policy = endpoint.policy().clone();
        let unix_config = ServerConfig::new(Arc::clone(&authority), identity);
        let unix = serve_unix(endpoint, unix_config, Arc::clone(&handler))?;
        let ws_endpoint = WsEndpoint::bind(WsBinding::default()).await?;
        let origins = AllowedOrigins::new([ORIGIN.parse()?]);
        let ws_config =
            WsConfig::new(Arc::clone(&authority), identity, policy.clone()).with_origins(origins);
        let ws = serve_websocket(ws_endpoint, ws_config, handler)?;
        let bearers = HashMap::from([("owner".to_owned(), issued.bearer().to_owned())]);
        Ok(Self {
            root,
            authority,
            policy,
            bearers: Mutex::new(bearers),
            unix: Some(unix),
            ws_address: ws.local_addr(),
            ws: Some(ws),
            hub,
            lane,
            core,
            target,
            raw: Vec::new(),
        })
    }

    /// Issues `permissions` on the terminal to `label`, valid until `expires_at`.
    pub(crate) fn issue(
        &self,
        label: &str,
        (permissions, expires_at): (&[Permission], u64),
    ) -> TestResult<String> {
        let owner_bearer = self.bearer("owner")?;
        let admission = local_admission()?;
        let now = TimestampMs::new(1_000);
        let owner = self.authority.authenticate(
            &admission,
            ovis_terminal_authz::Presentation::new(&owner_bearer, now),
        )?;
        let scope = ResourceScope::new(
            runtime()?.machine_id(),
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
            TimestampMs::new(expires_at),
        );
        let bearer = self
            .authority
            .issue(&owner, Issuance::new(request, now))?
            .bearer()
            .to_owned();
        self.bearers
            .lock()
            .map_err(|_| "bearers poisoned")?
            .insert(label.to_owned(), bearer.clone());
        Ok(bearer)
    }

    pub(crate) fn bearer(&self, label: &str) -> TestResult<String> {
        let bearers = self.bearers.lock().map_err(|_| "bearers poisoned")?;
        Ok(bearers
            .get(label)
            .cloned()
            .ok_or("principal was not issued")?)
    }

    /// A connection over `transport` that has not sent hello yet.
    pub(crate) async fn open(&self, transport: Transport) -> TestResult<Client> {
        Ok(match transport {
            Transport::Unix => Client::Unix(BufReader::new(
                UnixStream::connect(self.root.join("run").join("s.sock")).await?,
            )),
            Transport::Ws => Client::Ws(
                WsClient::connect(self.ws_address, None)
                    .await?
                    .map_err(|refused| format!("upgrade refused: {refused}"))?,
            ),
        })
    }

    /// A connection over `transport` authenticated as `label`.
    pub(crate) async fn connect(&self, label: &str, transport: Transport) -> TestResult<Client> {
        let mut client = self.open(transport).await?;
        let reply = client.hello(&self.bearer(label)?).await?;
        if reply.get("result").is_none() {
            return Err(format!("hello refused: {reply}").into());
        }
        Ok(client)
    }

    /// A second WebSocket server of the same authority whose upgrades must arrive
    /// within `deadline`.
    pub(crate) async fn idle_ws(&self, deadline: Duration) -> TestResult<WsServerHandle> {
        let config = WsConfig::new(Arc::clone(&self.authority), runtime()?, self.policy.clone())
            .with_handshake_timeout(deadline);
        let endpoint = WsEndpoint::bind(WsBinding::default()).await?;
        Ok(serve_websocket(endpoint, config, Arc::new(Nothing))?)
    }

    /// Feeds holder output.
    pub(crate) fn publish(&mut self, bytes: &[u8]) -> TestResult {
        let output = Output {
            target: self.target.clone(),
            offset: u64::try_from(self.raw.len())?,
            bytes: bytes.to_vec(),
            hangup: None,
        };
        self.raw.extend_from_slice(bytes);
        self.hub.publish(&output)?;
        Ok(())
    }

    /// Records the program's exit after every published byte.
    pub(crate) fn exit(&self) -> TestResult {
        let exit: Exit = serde_json::from_value(serde_json::json!({
            "target": self.target, "code": 0, "signal": null, "exit_unknown": null,
            "io_failure": null, "final_offset": self.raw.len().to_string(),
        }))?;
        Ok(self.hub.finish(&exit)?)
    }

    pub(crate) async fn close(mut self) -> TestResult<WsReport> {
        let report = match self.ws.take() {
            Some(ws) => ws.shutdown().await?,
            None => WsReport::default(),
        };
        if let Some(unix) = self.unix.take() {
            unix.shutdown().await?;
        }
        self.lane.shutdown().await?;
        drop((self.hub, self.lane));
        let core = Arc::try_unwrap(self.core).map_err(|_| "coordinator still shared")?;
        tokio::task::spawn_blocking(move || core.close()).await??;
        std::fs::remove_dir_all(&self.root)?;
        Ok(report)
    }
}
