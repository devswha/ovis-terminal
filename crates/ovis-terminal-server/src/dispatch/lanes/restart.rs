//! `server.restart` and `server.stop` over the restartable control (plan todo 48).
//!
//! Both are machine-scoped administrative mutations. `server.restart` hands holder
//! ownership to a successor control under a new boot identity while every program keeps
//! running; `server.stop` names its scope explicitly and has no default. Offered while a
//! restart is between boots, either is refused as `restarting` before any effect, so
//! the ledger records a failure, never an uncertain outcome. The served binary that
//! registers this lane and replaces its listener at the boundary is assembled by plan
//! todo 31.

use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_core::restart::{Control, RestartError};
use ovis_terminal_protocol::schema::methods::server::{ServerStartParams, ServerStopParams};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

#[cfg(test)]
mod qa;

/// The control both methods act on, and the machine they are authorized against.
#[derive(Debug)]
pub struct RestartLane {
    control: Arc<Control>,
    machine: MachineId,
}

impl RestartLane {
    /// Serves `control` of `machine`.
    #[must_use]
    pub fn new(control: Arc<Control>, machine: MachineId) -> Arc<Self> {
        Arc::new(Self { control, machine })
    }

    const fn access(&self) -> Access {
        Access::new(
            Action::Administer,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        )
    }

    fn ready(&self) -> Result<Arc<Control>, DispatchError> {
        self.control.ensure_ready().map_err(domain)?;
        Ok(Arc::clone(&self.control))
    }
}

fn domain(error: RestartError) -> DispatchError {
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

/// `server.restart`: a planned control restart that keeps every program incarnation.
#[derive(Debug)]
pub struct ServerRestart(Arc<RestartLane>);

impl ServerRestart {
    /// The method over `lane`.
    #[must_use]
    pub fn new(lane: &Arc<RestartLane>) -> Self {
        Self(Arc::clone(lane))
    }
}

/// `server.stop`: stops control, and its programs only when the scope says so.
#[derive(Debug)]
pub struct ServerStop(Arc<RestartLane>);

impl ServerStop {
    /// The method over `lane`.
    #[must_use]
    pub fn new(lane: &Arc<RestartLane>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<D, P> Method<D, P> for ServerRestart
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "server.restart";
    type Params = ServerStartParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access())
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let control = self.0.ready()?;
        execution
            .commit(move || {
                let receipt = control.restart(params.op_id).map_err(domain)?;
                Ok(receipt.transition())
            })
            .await
    }
}

impl<D, P> Method<D, P> for ServerStop
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "server.stop";
    type Params = ServerStopParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access())
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let control = self.0.ready()?;
        execution
            .commit(move || {
                let receipt = control.stop(params.op_id, params.scope).map_err(domain)?;
                Ok(receipt.transition())
            })
            .await
    }
}
