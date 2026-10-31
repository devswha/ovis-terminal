//! Running requests and operation-control calls against the landed ledger.

use std::sync::Arc;

use ovis_terminal_authz::{Decision, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{
    Context, OperationError, OperationEvent, OperationSnapshot, Operations,
};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::watch;

use super::execution::{State, now};
use super::registry::Prepared;
use super::{Access, DispatchError, Execution};

pub(super) struct Job<D, P> {
    pub state: Arc<State<D, P>>,
    pub session: Arc<Session>,
    pub decision: Decision<RegistryIdentities>,
    pub notifier: crate::conn::Notifier,
}

impl<D, P> Job<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    pub(super) async fn run(
        self,
        prepared: Prepared<D, P>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<Value, BoundaryError> {
        let op_id = prepared.request.op_id().clone();
        self.perform(prepared, cancellation)
            .await
            .map_err(|error| error.boundary().with_op_id(op_id))
    }

    async fn perform(
        self,
        prepared: Prepared<D, P>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<Value, DispatchError> {
        let context = Context::new(
            self.session.principal().clone(),
            self.state.identity,
            now()?,
        );
        let state = Arc::clone(&self.state);
        let accepted_context = context.clone();
        let record = tokio::task::spawn_blocking(move || {
            Operations::new(&state.core, accepted_context).accept(&prepared.request)
        })
        .await??;
        match record.state {
            OpenEnum::Known(OpState::Accepted) => {}
            OpenEnum::Known(OpState::Committed) => {
                return record
                    .result
                    .ok_or_else(|| OperationError::InvalidLedger.into());
            }
            OpenEnum::Known(_) | OpenEnum::Unknown(_) => {
                return Err(OperationError::Settled(Box::new(record)).into());
            }
        }
        let op_id = record.op_id;
        let execution = Execution {
            state: Arc::clone(&self.state),
            context: context.clone(),
            op_id: op_id.clone(),
            access: prepared.access,
            decision: self.decision,
            cancellation,
            notifier: self.notifier,
        };
        match (prepared.run)(execution).await {
            Ok(completed) => Ok(completed.0),
            Err(error) => {
                let state = self.state;
                let data = error.boundary().data().clone();
                tokio::task::spawn_blocking(move || {
                    let operations = Operations::new(&state.core, context.at(now()?));
                    let record = operations.get(&op_id)?;
                    match record.state {
                        OpenEnum::Known(OpState::Accepted) => {
                            operations.fail(&op_id, data)?;
                        }
                        OpenEnum::Known(
                            OpState::Running
                            | OpState::Committed
                            | OpState::Failed
                            | OpState::Cancelled
                            | OpState::OutcomeUnknown,
                        ) => {}
                        OpenEnum::Known(_) | OpenEnum::Unknown(_) => {
                            return Err(OperationError::InvalidLedger.into());
                        }
                    }
                    Ok::<_, DispatchError>(())
                })
                .await??;
                Err(error)
            }
        }
    }

    pub(super) async fn control(
        self,
        request: Control,
        signals: Vec<watch::Sender<bool>>,
    ) -> Result<Value, BoundaryError> {
        let op_id = request.op_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let at = now()?;
            self.state
                .authority
                .recheck(&self.decision, &request.access.at(at))?;
            let operations = Operations::new(
                &self.state.core,
                Context::new(self.session.principal().clone(), self.state.identity, at),
            );
            let value = if request.cancel {
                let result = operations.cancel(&request.op_id)?;
                for signal in signals {
                    signal.send_replace(true);
                }
                serde_json::to_value(result)?
            } else {
                serde_json::to_value(operations.get(&request.op_id)?)?
            };
            Ok::<_, DispatchError>(value)
        })
        .await;
        result
            .map_err(DispatchError::from)
            .and_then(std::convert::identity)
            .map_err(|error| error.boundary().with_op_id(op_id))
    }
}

pub(super) struct Control {
    pub op_id: OpId,
    pub access: Access,
    pub cancel: bool,
}
