//! Run one execution and retain its outcome in the landed operation ledger.

use std::sync::Arc;

use ovis_terminal_authz::{Decision, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{
    Context, OperationError, OperationEvent, OperationSnapshot, Operations,
};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::watch;

use super::execution::{State, now};
use super::queue::{Admission, WithdrawOnDrop};
use super::registry::Prepared;
use super::reservation::Reservation;
use super::{DispatchError, Execution};

pub(super) struct Job<D, P> {
    pub state: Arc<State<D, P>>,
    pub session: Arc<Session>,
    pub decision: Decision<RegistryIdentities>,
    pub notifier: crate::conn::Notifier,
    pub reservation: Arc<Reservation>,
    pub admission: Admission,
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
        let reservation = Arc::clone(&self.reservation);
        let request = Arc::clone(&prepared.request);
        let Admission {
            ticket,
            permit,
            queued,
        } = self.admission;
        let abandoned = WithdrawOnDrop(queued);
        let record = tokio::task::spawn_blocking(move || {
            // Waits in FIFO order; the connection's queue place is freed once admitted.
            let slot = ticket.wait().map_err(OperationError::from)?;
            drop(permit);
            let operations = Operations::new(&state.core, accepted_context);
            let acceptance = reservation.accept();
            let accepted =
                if acceptance.cancel_requested() {
                    match operations.accept_cancelled_granted(&request, slot) {
                        Ok(cancelled) => {
                            acceptance.deliver(serde_json::to_value(&cancelled).map_err(|error| {
                                DispatchError::from(error).boundary().data().clone()
                            }));
                            Ok(cancelled.operation)
                        }
                        Err(error) => {
                            let error = DispatchError::from(error);
                            acceptance.deliver(Err(error.boundary().data().clone()));
                            Err(error)
                        }
                    }
                } else {
                    operations
                        .accept_granted(&request, slot)
                        .map_err(DispatchError::from)
                };
            drop(acceptance);
            accepted
        })
        .await;
        drop(abandoned);
        let record = record??;
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
            session: Arc::clone(&self.session),
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
}
