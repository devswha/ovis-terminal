//! `server.status` and `server.metrics` over the core health tables (plan todo 59).
//!
//! Both are unrecorded reads of one committed snapshot: authorized at observe level on
//! the machine, never admitted to the operation ledger, and answered from values the
//! runtime already published, so a read writes no receipt, acknowledges no recovery
//! failure and starts nothing. Their parameters are the closed empty object; a request
//! such as `{"reset": true}` is `invalid_params` before anything is read. [`Measured`]
//! wraps the connection handler and records each call's latency once it settles; a call
//! still pending holds no lock a metrics read needs.

use std::sync::Arc;
use std::time::Instant;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::health::{Health, Observed, Settled};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{MachineId, RuntimeIdentity};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, OpId};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{Snapshot, StoreError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::conn::{Call, RequestHandler};
use crate::dispatch::{Access, DispatchError, Dispatcher, Read};

#[cfg(test)]
mod qa;

const fn machine_read(machine: MachineId) -> Access {
    Access::new(
        Action::Read,
        ResolvedTarget::new(machine, TargetResource::Machine),
    )
}

fn failed(kind: &'static str, hint: &'static str) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(kind.to_owned()), false).with_hint(hint)
}

fn store_failed(error: &StoreError) -> BoundaryError {
    failed(error.kind(), error.hint())
}

fn encoded(record: &impl Serialize) -> Result<Value, BoundaryError> {
    serde_json::to_value(record)
        .map_err(|_| failed("internal", "report the health record that failed to encode"))
}

/// `server.status`: this server's own state, which is running because it answers.
#[derive(Debug)]
pub struct Status {
    health: Arc<Health>,
    machine: MachineId,
}

/// `server.metrics`: per-consumer backlog, latency and recovery state of this boot.
#[derive(Debug)]
pub struct Metrics {
    health: Arc<Health>,
    machine: MachineId,
}

impl Read for Status {
    const NAME: &'static str = "server.status";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(machine_read(self.machine))
    }

    fn read(&self, _: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let quarantined = snapshot
            .recovery_evidence()
            .map_err(|error| store_failed(&error))?;
        encoded(&self.health.status(&quarantined))
    }
}

impl Read for Metrics {
    const NAME: &'static str = "server.metrics";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(machine_read(self.machine))
    }

    fn read(&self, _: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let quarantined = snapshot
            .recovery_evidence()
            .map_err(|error| store_failed(&error))?;
        let observed = Observed::new(EventSeq::new(snapshot.revision().get()), quarantined);
        let metrics = self
            .health
            .metrics(&observed)
            .map_err(|error| failed(error.kind(), error.hint()))?;
        encoded(&metrics)
    }
}

/// Registers `server.status` and `server.metrics` for `runtime`'s machine.
///
/// # Errors
///
/// Returns the registration failure when either method is already registered.
pub fn register<D, P>(
    dispatch: &mut Dispatcher<D, P>,
    health: &Arc<Health>,
    runtime: RuntimeIdentity,
) -> Result<(), DispatchError>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    let machine = runtime.machine_id();
    dispatch.register_read(Status {
        health: Arc::clone(health),
        machine,
    })?;
    dispatch.register_read(Metrics {
        health: Arc::clone(health),
        machine,
    })
}

/// Records the latency of every call `inner` settles.
#[derive(Debug)]
pub struct Measured<H> {
    health: Arc<Health>,
    inner: Arc<H>,
}

impl<H> Measured<H> {
    /// Measures `inner` into `health`'s latency table.
    #[must_use]
    pub const fn new(health: Arc<Health>, inner: Arc<H>) -> Self {
        Self { health, inner }
    }
}

impl<H: RequestHandler> RequestHandler for Measured<H> {
    fn methods(&self) -> Vec<&'static str> {
        self.inner.methods()
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        let method = call.request.method().to_owned();
        let op_id = call
            .request
            .params()
            .get("op_id")
            .and_then(Value::as_str)
            .map(|op| OpId::new(op.to_owned()));
        let started = Instant::now();
        let reply = self.inner.call(call).await;
        let settled = Settled::new((&method, started.elapsed()), (reply.is_err(), op_id));
        self.health.latencies().record(settled);
        reply
    }
}
