//! The RPC wait expires independently of its owned operation task.

use std::future::Future;
use std::time::Duration;

use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, OpId};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use serde_json::Value;
use tokio::sync::watch;
use tokio::time::Instant;

use super::DispatchError;
use super::queue::Queued;

#[cfg(test)]
mod tests;

pub(super) async fn wait(
    received: impl Future<Output = Result<Value, BoundaryError>> + Send,
    op_id: OpId,
    start: Instant,
) -> Result<Value, BoundaryError> {
    wait_queued(received, (op_id, None), start).await
}

/// [`wait`] whose deadline failure carries the request's admission position while queued.
pub(super) async fn wait_queued(
    received: impl Future<Output = Result<Value, BoundaryError>> + Send,
    (op_id, queued): (OpId, Option<Queued>),
    start: Instant,
) -> Result<Value, BoundaryError> {
    let deadline = Limits::v1().rpc_default_deadline_ms;
    let expires = start
        .checked_add(Duration::from_millis(deadline.get()))
        .ok_or_else(|| {
            DispatchError::DeadlineOverflow
                .boundary()
                .with_op_id(op_id.clone())
        })?;
    let outcome = tokio::time::timeout_at(expires, received).await;
    let settled = Instant::now();
    match outcome {
        Ok(result) if settled < expires => result.map_err(|error| error.with_op_id(op_id)),
        Ok(_) | Err(_) => {
            let elapsed =
                u64::try_from(settled.duration_since(start).as_millis()).unwrap_or(u64::MAX);
            let failure =
                BoundaryError::deadline_exceeded(op_id, deadline, DurationMs::new(elapsed));
            Err(match queued.as_ref().and_then(Queued::position) {
                Some(position) => failure.with_queued_position(position),
                None => failure,
            })
        }
    }
}

pub(super) type Reply = Result<Value, ErrorData>;
pub(super) type Response = watch::Receiver<Option<Reply>>;

pub(super) async fn receive(mut response: Response) -> Result<Value, BoundaryError> {
    let reply = response
        .wait_for(Option::is_some)
        .await
        .map_err(|_| DispatchError::Closed.boundary())?;
    match reply.as_ref() {
        Some(Ok(value)) => Ok(value.clone()),
        Some(Err(data)) => Err(BoundaryError::Domain {
            data: Box::new(data.clone()),
        }),
        None => Err(DispatchError::Closed.boundary()),
    }
}
