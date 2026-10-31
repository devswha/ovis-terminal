//! The authorized effect boundary, separate from an RPC's reply lifetime.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{AccessRequest, Action, Authority, Decision, ResolvedTarget};
use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationEvent, OperationSnapshot, Operations,
    RunningOperation,
};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::watch;

use super::DispatchError;
use crate::conn::Notifier;

/// A domain's resolved action and target, never a focus-based default.
#[derive(Debug, Clone)]
pub struct Access {
    pub(super) action: Action,
    pub(super) target: ResolvedTarget<RegistryIdentities>,
}

impl Access {
    /// Binds a method's action to the resource its owning registry resolved.
    pub const fn new(action: Action, target: ResolvedTarget<RegistryIdentities>) -> Self {
        Self { action, target }
    }

    pub(super) fn at(&self, now: TimestampMs) -> AccessRequest<RegistryIdentities> {
        AccessRequest::new(self.action, self.target.clone(), now)
    }
}

/// A result whose effect and durable receipt have passed the commit boundary.
#[derive(Debug)]
pub struct Completed(pub(super) Value);

#[derive(Debug)]
pub(super) struct State<D, P> {
    pub authority: Arc<Authority<RegistryIdentities>>,
    pub identity: RuntimeIdentity,
    pub core: Arc<Coordinator<D, P>>,
}

/// One accepted operation's single-use commit permission.
///
/// A domain may prepare asynchronously, but effects belong inside [`Execution::commit`].
/// Cancellation must be observed during preparation with [`Execution::cancelled`].
/// Once the effect starts it must complete or report uncertainty; a deadline never drops it.
#[derive(Debug)]
pub struct Execution<D, P> {
    pub(super) state: Arc<State<D, P>>,
    pub(super) context: Context,
    pub(super) op_id: OpId,
    pub(super) access: Access,
    pub(super) decision: Decision<RegistryIdentities>,
    pub(super) cancellation: watch::Receiver<bool>,
    pub(super) notifier: Notifier,
}

impl<D, P> Execution<D, P> {
    /// Resolves when the caller requests cancellation or dispatch begins shutdown.
    pub async fn cancelled(&self)
    where
        D: Send,
        P: Send + Sync,
    {
        let mut cancellation = self.cancellation.clone();
        // A closed signal also means the owning dispatcher has gone.
        match cancellation.wait_for(|cancelled| *cancelled).await {
            Ok(_) | Err(_) => {}
        }
    }

    /// The operation's inspectable identity.
    pub const fn op_id(&self) -> &OpId {
        &self.op_id
    }

    /// The exact resolved resource covered by the authorization decision.
    pub const fn target(&self) -> &ResolvedTarget<RegistryIdentities> {
        &self.access.target
    }

    /// The originating connection's bounded notification outlet.
    pub const fn notifier(&self) -> &Notifier {
        &self.notifier
    }
}

impl<D, P> Execution<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Rechecks authority and cancellation immediately before granting effect permission.
    ///
    /// Runs blocking effects and persistence away from Tokio's executor. The caller owns
    /// this task until it finishes, including after the reply deadline or disconnect.
    ///
    /// # Errors
    ///
    /// Returns authority, cancellation, encoding or persistence failures. Effect and
    /// encoding failures record `outcome_unknown` when persistence remains available;
    /// persistence failures retain the operation identity, never an unproven rollback.
    pub async fn commit<F, R>(self, effect: F) -> Result<Completed, DispatchError>
    where
        F: FnOnce() -> Result<R, DispatchError> + Send + 'static,
        R: Serialize + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let operations = Operations::new(&self.state.core, self.context.clone().at(now()?));
            let permission = self.permit(&operations)?;
            match effect().and_then(|result| serde_json::to_value(result).map_err(Into::into)) {
                Ok(result) => {
                    operations.commit(permission, result.clone())?;
                    Ok(Completed(result))
                }
                Err(error) => {
                    operations.outcome_unknown(permission)?;
                    Err(error)
                }
            }
        })
        .await?
    }

    /// Commits the method's state writes, its domain events and the operation receipt in
    /// one SQLite transaction built from one read snapshot.
    ///
    /// A [`Completion::failed`] outcome is retained as the operation's typed failure and
    /// returned as that error, so a retry with the same operation id answers it again.
    ///
    /// # Errors
    ///
    /// Returns the retained domain failure, or authority, cancellation and persistence
    /// failures; a rolled-back unit leaves the operation running, never committed.
    pub async fn commit_unit<F>(self, build: F) -> Result<Completed, DispatchError>
    where
        F: FnOnce(&Snapshot) -> Result<Completion<D>, OperationError> + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let operations = Operations::new(&self.state.core, self.context.clone().at(now()?));
            let permission = self.permit(&operations)?;
            let record = operations.commit_unit(permission, build)?;
            match (&record.state, record.result.clone()) {
                (OpenEnum::Known(OpState::Committed), Some(result)) => Ok(Completed(result)),
                _ => Err(OperationError::Settled(Box::new(record)).into()),
            }
        })
        .await?
    }

    /// Starts the durable intent, then rechecks authority at the effect boundary.
    fn permit(&self, operations: &Operations<'_, D, P>) -> Result<RunningOperation, DispatchError> {
        if *self.cancellation.borrow() {
            operations.cancel(&self.op_id)?;
        }
        let permission = operations.start(&self.op_id)?;
        // Starting the durable intent can block behind SQLite; revoke/expiry must
        // therefore be checked after that wait, at the actual effect boundary.
        if let Err(error) = self
            .state
            .authority
            .recheck(&self.decision, &self.access.at(now()?))
        {
            let error = DispatchError::Authority(error);
            operations.fail_running(permission, error.boundary().data().clone())?;
            return Err(error);
        }
        Ok(permission)
    }
}

impl<D, P> Execution<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
}

pub(super) fn now() -> Result<TimestampMs, DispatchError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(TimestampMs::new(u64::try_from(elapsed.as_millis())?))
}
