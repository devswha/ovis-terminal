//! IDEAL-010 for the lifecycle mutations the runtime's actor serves: a caller learns the
//! operation's id by the deadline even when the actor is busy. The work stays queued on
//! the actor and settles in the lifecycle ledger, where `op.get` reads it.

use std::future::Future;
use std::time::Duration;

use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use ovis_terminal_server::conn::Call;
use serde_json::Value;
use tokio::time::Instant;

/// The mutations the actor runs that are recorded in the lifecycle ledger.
const LIFECYCLE: [&str; 7] = [
    "terminal.run",
    "terminal.kill",
    "terminal.rm",
    "terminal.keep",
    "terminal.rename",
    "terminal.label",
    "terminal.recover",
];

/// The operation id of a lifecycle mutation, which the deadline then applies to.
pub(super) fn lifecycle_op(call: &Call) -> Option<OpId> {
    if !LIFECYCLE.contains(&call.request.method()) {
        return None;
    }
    call.request
        .params()
        .get("op_id")
        .and_then(Value::as_str)
        .map(|op| OpId::new(op.to_owned()))
}

/// Answers `routed`, or at the deadline `deadline_exceeded` carrying `op`.
pub(super) async fn within(
    routed: impl Future<Output = Result<Value, BoundaryError>>,
    op: OpId,
) -> Result<Value, BoundaryError> {
    let deadline = Limits::v1().rpc_default_deadline_ms;
    let started = Instant::now();
    let answer = tokio::time::timeout(Duration::from_millis(deadline.get()), routed).await;
    answer.unwrap_or_else(|_| {
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Err(BoundaryError::deadline_exceeded(
            op,
            deadline,
            DurationMs::new(elapsed),
        ))
    })
}
