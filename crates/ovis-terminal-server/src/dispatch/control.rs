//! Ledger-owner resolution precedes authority decisions for operation control.

use std::sync::Arc;

use ovis_terminal_authz::{Action, PrincipalId, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{Context, OperationEvent, OperationSnapshot, Operations};
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::deadline::receive;
use super::execution::{State, now};
use super::queue::Queued;
use super::reservation::Reservation;
use super::{Access, DispatchError};

pub(super) struct Control {
    pub op_id: OpId,
    pub cancel: bool,
    /// The caller's own dispatched request for this identity, if it is still owned.
    pub reservation: Option<Arc<Reservation>>,
    /// Another principal's dispatched request, which owns the identity for authorization.
    pub foreign: Option<PrincipalId>,
    /// The caller's own dispatched request's place in the admission queue.
    pub queued: Option<Queued>,
}

impl Control {
    fn authorize<D, P>(
        &self,
        state: &State<D, P>,
        (session, owner): (&Session, PrincipalId),
    ) -> Result<(), DispatchError> {
        let access = Access::new(
            if self.cancel {
                Action::CancelOperation
            } else {
                Action::InspectOperation
            },
            ResolvedTarget::new(
                state.identity.machine_id(),
                TargetResource::Operation {
                    principal: owner,
                    op_id: self.op_id.clone(),
                },
            ),
        );
        let decision = state.authority.authorize(session, &access.at(now()?))?;
        state.authority.recheck(&decision, &access.at(now()?))?;
        Ok(())
    }
}

pub(super) async fn run<D, P>(
    state: Arc<State<D, P>>,
    session: Arc<Session>,
    request: Control,
) -> Result<Value, BoundaryError>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    let op_id = request.op_id.clone();
    // A request still waiting for admission has no ledger row yet; its queue place is
    // the truthful answer, and reading the ledger would queue behind it.
    if !request.cancel
        && let Some(queued) = &request.queued
        && let Some(position) = queued.position()
    {
        request
            .authorize(&state, (&session, session.principal().clone()))
            .map_err(|error| error.boundary().with_op_id(op_id.clone()))?;
        let record = OpRecord::queued(op_id.clone(), queued.method.clone(), position);
        return serde_json::to_value(record)
            .map_err(|error| DispatchError::from(error).boundary().with_op_id(op_id));
    }
    if request.cancel
        && let Some(reservation) = &request.reservation
    {
        request
            .authorize(&state, (&session, session.principal().clone()))
            .map_err(|error| error.boundary().with_op_id(op_id.clone()))?;
        // Before the durable accept, the job delivers this cancellation as it commits.
        if let Some(delivered) = reservation.cancel().await {
            return receive(delivered).await;
        }
    }
    let result = tokio::task::spawn_blocking(move || {
        let operations = Operations::new(
            &state.core,
            Context::new(session.principal().clone(), state.identity, now()?),
        );
        let owner = operations
            .owner(&request.op_id)?
            .or_else(|| request.foreign.clone())
            .unwrap_or_else(|| session.principal().clone());
        request.authorize(&state, (&session, owner))?;
        let value = if request.cancel {
            serde_json::to_value(operations.cancel(&request.op_id)?)?
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
