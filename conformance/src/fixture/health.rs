//! `server.status` and `server.metrics` on the real dispatcher and latency wrapper.
//!
//! Three unadvertised stand-ins cover what this fixture does not run: `fixture.viewer` reports a stalled attach viewer into the real consumer table (no
//! attach stream is served until plan todo 31), `fixture.incident` records a durable
//! incident through the real journal, and `fixture.stale_endpoint` leaves a dead socket
//! at the default endpoint, the trace a crashed server leaves behind.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ovis_terminal_core::health::{ConsumerGauge, Health, RecoveryLedger};
use ovis_terminal_core::recovery::{Finding, Incidents};
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::domain::health::{ConsumerRef, LossRange};
use ovis_terminal_protocol::domain::recovery::IncidentKind;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::ids::{ClientId, ServerBootId, StreamId, TerminalId};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};
use ovis_terminal_server::dispatch::lanes::health;
use ovis_terminal_server::listen::{EndpointDescriptor, EndpointEnvironment, EnvVar, Platform};
use serde::Deserialize;
use serde_json::{Value, json};

use super::handler::Dispatch;
use super::serve::at;
use crate::error::RunnerError;

/// Reports a stalled viewer; served but never advertised.
pub(crate) const VIEWER: &str = "fixture.viewer";
/// Records a recovery incident; served but never advertised.
pub(crate) const INCIDENT: &str = "fixture.incident";
/// Leaves a dead socket at the default endpoint; served but never advertised.
pub(crate) const STALE_ENDPOINT: &str = "fixture.stale_endpoint";

/// The fixture's health and the gauges its stand-in viewers keep alive.
#[derive(Debug)]
pub(crate) struct Probe {
    pub(crate) health: Arc<Health>,
    root: PathBuf,
    viewers: Mutex<Vec<ConsumerGauge>>,
}

/// Opens the incident journal under `root`, registers both reads and returns the probe.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    runtime: RuntimeIdentity,
    root: &Path,
) -> Result<Arc<Probe>, RunnerError> {
    let boot = ServerBootId::new(runtime.server_boot_id().to_string());
    let journal = Incidents::new(&root.join("recovery"));
    let ledger = RecoveryLedger::open(journal, (boot, TimestampMs::new(0)), None)
        .map_err(at("open the recovery ledger"))?;
    let health = Arc::new(Health::new(runtime, ledger));
    health::register(dispatch, &health, runtime).map_err(at("register the health reads"))?;
    Ok(Arc::new(Probe {
        health,
        root: root.to_path_buf(),
        viewers: Mutex::new(Vec::new()),
    }))
}

fn refused(hint: &'static str) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from("internal".to_owned()), false).with_hint(hint)
}

fn params<T: for<'de> Deserialize<'de>>(request: &RpcRequest) -> Result<T, BoundaryError> {
    serde_json::from_value(request.params().clone())
        .map_err(|_| refused("send the fixture probe's documented params"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stall {
    stream_id: StreamId,
    terminal_id: TerminalId,
    queued: crate::ids::Decimal,
    lost: [ByteOffset; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Incident {
    kind: IncidentKind,
}

impl Probe {
    /// Answers one stand-in request.
    pub(crate) fn call(&self, request: &RpcRequest) -> Result<Value, BoundaryError> {
        match request.method() {
            VIEWER => self.stall(params(request)?),
            INCIDENT => {
                let Incident { kind } = params(request)?;
                let incident = self
                    .health
                    .recovery()
                    .record(Finding::new(kind, Vec::new(), None))
                    .map_err(|error| refused(error.hint()))?;
                Ok(json!({ "incident": incident }))
            }
            _ => self.stale_endpoint(),
        }
    }

    /// A viewer that stopped reading: `queued` bytes stay unwritten, the overflowing
    /// range `lost` is dropped, and the viewer is resynchronized past it from a snapshot.
    fn stall(&self, stall: Stall) -> Result<Value, BoundaryError> {
        let consumer = ConsumerRef::Stream {
            stream_id: stall.stream_id.clone(),
            terminal_id: stall.terminal_id,
        };
        let gauge = self
            .health
            .consumers()
            .track(ClientId::new("fixture-viewer".to_owned()), consumer)
            .map_err(|error| refused(error.hint()))?;
        gauge.queued(stall.queued.0, TimestampMs::new(0));
        let [first_offset, last_offset] = stall.lost;
        let range = LossRange::Bytes {
            first_offset,
            last_offset,
        };
        gauge.lost(range);
        gauge.resynced(range);
        self.viewers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(gauge);
        Ok(json!({ "stream_id": stall.stream_id }))
    }

    /// Binds the default endpoint of this root and closes the listener without removing
    /// the socket file, so a later connection is refused.
    fn stale_endpoint(&self) -> Result<Value, BoundaryError> {
        let env = EndpointEnvironment::default()
            .with(EnvVar::RuntimeDir, self.root.as_os_str())
            .with(EnvVar::Home, self.root.as_os_str());
        let descriptor = EndpointDescriptor::resolve(&env, Platform::CURRENT)
            .map_err(|_| refused("the default endpoint of the root does not resolve"))?;
        let path = descriptor
            .unix_path()
            .ok_or_else(|| refused("the default endpoint is not a Unix socket"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|_| refused("create the endpoint directory"))?;
        }
        drop(
            std::os::unix::net::UnixListener::bind(path)
                .map_err(|_| refused("bind the default endpoint"))?,
        );
        Ok(json!({ "socket": path }))
    }
}
