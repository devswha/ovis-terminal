//! The RPC wait expires independently of its owned operation task.

use std::time::Duration;

use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::DispatchError;

pub(super) async fn wait(
    received: oneshot::Receiver<Result<Value, BoundaryError>>,
    op_id: OpId,
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
    match tokio::time::timeout_at(expires, received).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(DispatchError::Closed.boundary().with_op_id(op_id)),
        Err(_) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            Err(BoundaryError::deadline_exceeded(
                op_id,
                deadline,
                DurationMs::new(elapsed),
            ))
        }
    }
}
