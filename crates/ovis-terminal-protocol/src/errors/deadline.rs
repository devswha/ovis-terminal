//! Deadline settlement does not cancel the inspectable operation.

use super::{BoundaryError, ErrorData, ErrorKind, OpId};
use crate::schema::scalar::DurationMs;

impl BoundaryError {
    /// Ends the RPC wait without abandoning the operation named by `op_id`.
    pub fn deadline_exceeded(op_id: OpId, deadline_ms: DurationMs, elapsed_ms: DurationMs) -> Self {
        let mut data = ErrorData::new(ErrorKind::from("deadline_exceeded".to_owned()), false);
        "inspect or cancel the operation by op_id".clone_into(&mut data.hint);
        data.op_id = Some(op_id);
        data.deadline_ms = Some(deadline_ms);
        data.elapsed_ms = Some(elapsed_ms);
        Self::Domain {
            data: Box::new(data),
        }
    }
}
