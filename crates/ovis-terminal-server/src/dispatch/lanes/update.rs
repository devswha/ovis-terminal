//! The `update.*` methods over the update service (plan todo 47).
//!
//! Every mutation is a machine-scoped administrative operation. Refusals whose outcome
//! is known (a digest mismatch, a rejected build, an activation that selected the
//! previous build again) settle the operation `failed` with their typed kind, so
//! `op.get` and `update.query` both show what happened; only install-root I/O and
//! control failures are recorded `outcome_unknown`. `update.query` is an unrecorded read.

use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::methods::update::{UpdateStageParams, UpdateStepParams};
use ovis_terminal_services::update::{Interrupt, UpdateError, Updates};
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::dispatch::{
    Access, Completed, DispatchError, Dispatcher, Execution, Method, Read, Unsettled,
};

#[cfg(unix)]
pub mod control;
#[cfg(all(test, unix))]
mod qa;

/// The update service and the machine its methods are authorized against.
#[derive(Debug, Clone)]
pub struct UpdateLane {
    service: Arc<Updates>,
    machine: MachineId,
}

fn domain(error: UpdateError) -> DispatchError {
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

fn unsettled(error: UpdateError) -> Unsettled {
    if error.proven() {
        Unsettled::Proven(domain(error))
    } else {
        Unsettled::Uncertain(domain(error))
    }
}

impl UpdateLane {
    /// Serves `service` for `machine`.
    #[must_use]
    pub const fn new(service: Arc<Updates>, machine: MachineId) -> Self {
        Self { service, machine }
    }

    const fn access(&self, action: Action) -> Access {
        Access::new(
            action,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        )
    }

    /// Registers the four mutations and the `update.query` read.
    ///
    /// # Errors
    ///
    /// A registration failure when a method is already registered.
    pub fn register<D, P>(self, dispatcher: &mut Dispatcher<D, P>) -> Result<(), DispatchError>
    where
        D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
        P: SnapshotProducer + Send + Sync + 'static,
        P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
    {
        dispatcher.register(UpdateStage(self.clone()))?;
        dispatcher.register(UpdateVerify(self.clone()))?;
        dispatcher.register(UpdateActivate(self.clone()))?;
        dispatcher.register(UpdateRollback(self.clone()))?;
        dispatcher.register_read(UpdateQuery(self))
    }
}

/// `update.stage`.
#[derive(Debug)]
pub struct UpdateStage(UpdateLane);

impl<D, P> Method<D, P> for UpdateStage
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "update.stage";
    type Params = UpdateStageParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Administer))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let service = Arc::clone(&self.0.service);
        let interrupt = Interrupt::new(execution.cancellation());
        execution
            .commit_proven(move || service.stage(&params, &interrupt).map_err(unsettled))
            .await
    }
}

/// Implements one step on a staged update.
macro_rules! step {
    ($(#[$doc:meta])* $method:ident, $name:literal, $step:ident) => {
        $(#[$doc])*
        #[derive(Debug)]
        pub struct $method(UpdateLane);

        impl<D, P> Method<D, P> for $method
        where
            D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
            D: Send + 'static,
            P: SnapshotProducer + Send + Sync + 'static,
            P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
        {
            const NAME: &'static str = $name;
            type Params = UpdateStepParams;

            fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
                Ok(self.0.access(Action::Administer))
            }

            async fn run(
                &self,
                params: Self::Params,
                execution: Execution<D, P>,
            ) -> Result<Completed, DispatchError> {
                let service = Arc::clone(&self.0.service);
                execution
                    .commit_proven(move || service.$step(&params).map_err(unsettled))
                    .await
            }
        }
    };
}

step!(
    /// `update.verify`.
    UpdateVerify,
    "update.verify",
    verify
);
step!(
    /// `update.activate`.
    UpdateActivate,
    "update.activate",
    activate
);
step!(
    /// `update.rollback`.
    UpdateRollback,
    "update.rollback",
    rollback
);

/// `update.query`, answered from the install root rather than the event snapshot.
#[derive(Debug)]
pub struct UpdateQuery(UpdateLane);

impl Read for UpdateQuery {
    const NAME: &'static str = "update.query";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Read))
    }

    fn read(&self, _: Self::Params, _: &Snapshot) -> Result<Value, BoundaryError> {
        let refused = |error: UpdateError| {
            BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
                .with_hint(error.hint())
        };
        let result = self.0.service.query().map_err(refused)?;
        serde_json::to_value(result).map_err(|_| {
            BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
                .with_hint("report this runtime build as defective")
        })
    }
}
