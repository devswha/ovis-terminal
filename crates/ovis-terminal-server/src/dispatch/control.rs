//! Ledger-owner resolution precedes authority decisions for operation control.

use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{Context, OperationEvent, OperationSnapshot, Operations};
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::watch;

use super::execution::{State, now};
use super::{Access, DispatchError};

pub(super) struct Control {
    pub op_id: OpId,
    pub cancel: bool,
    pub signals: Vec<watch::Sender<bool>>,
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
    let result = tokio::task::spawn_blocking(move || {
        let operations = Operations::new(
            &state.core,
            Context::new(session.principal().clone(), state.identity, now()?),
        );
        let owner = operations
            .owner(&request.op_id)?
            .unwrap_or_else(|| session.principal().clone());
        let access = Access::new(
            if request.cancel {
                Action::CancelOperation
            } else {
                Action::InspectOperation
            },
            ResolvedTarget::new(
                state.identity.machine_id(),
                TargetResource::Operation {
                    principal: owner,
                    op_id: request.op_id.clone(),
                },
            ),
        );
        let decision = state.authority.authorize(&session, &access.at(now()?))?;
        state.authority.recheck(&decision, &access.at(now()?))?;
        let value = if request.cancel {
            let result = operations.cancel(&request.op_id)?;
            for signal in request.signals {
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
