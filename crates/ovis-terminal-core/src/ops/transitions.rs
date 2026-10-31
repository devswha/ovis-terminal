//! Effect permission, completion and truthful cancellation.

use ovis_terminal_protocol::errors::{ErrorData, OpId};
use ovis_terminal_protocol::schema::methods::server::{
    CancelOutcome, OpCancelResult, OpRecord, OpState,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::admission::{Admission, Admitted};
use super::{
    OperationError, OperationEvent, OperationSnapshot, Operations, Request, RunningOperation,
};
use crate::events::{EventData, SnapshotProducer, SnapshotSlot};

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Commits intent before granting the caller permission to perform an external effect.
    ///
    /// # Errors
    ///
    /// Returns a receipt-bearing rejection for any operation already started or settled.
    pub fn start(&self, op: &OpId) -> Result<RunningOperation, OperationError> {
        let record = self.get(op)?;
        match record.state {
            OpenEnum::Known(OpState::Accepted) => {
                let mut row = self.current(op)?;
                if row.entry.record.state != OpenEnum::Known(OpState::Accepted) {
                    return Err(OperationError::Settled(Box::new(row.entry.record)));
                }
                row.entry.boot = self.context.identity.server_boot_id();
                row.entry.record.state = OpenEnum::Known(OpState::Running);
                self.update(row)?;
                Ok(RunningOperation {
                    op_id: op.clone(),
                    context: self.context.clone(),
                })
            }
            _ => Err(OperationError::Settled(Box::new(record))),
        }
    }

    /// Commits a proven method result without claiming to roll back external writes.
    ///
    /// # Errors
    ///
    /// Returns typed CAS, receipt-size or stale-permission errors.
    pub fn commit(
        &self,
        permit: RunningOperation,
        result: Value,
    ) -> Result<OpRecord, OperationError> {
        self.commit_unit(permit, |_snapshot| Ok(super::Completion::new(result)))
    }

    /// Retains a failure known to have occurred before external effects.
    ///
    /// # Errors
    ///
    /// Returns a receipt-bearing rejection if work has already started.
    pub fn fail(&self, op: &OpId, error: ErrorData) -> Result<OpRecord, OperationError> {
        let row = self.current(op)?;
        match row.entry.record.state {
            OpenEnum::Known(OpState::Accepted) => self.store_failure(row, error),
            _ => Err(OperationError::Settled(Box::new(row.entry.record))),
        }
    }

    /// Retains a proven failure after intent but before any external effect occurred.
    ///
    /// The caller must use `outcome_unknown` instead when the external effect may
    /// have begun or its absence cannot be proven.
    ///
    /// # Errors
    ///
    /// Returns typed persistence errors or a stale-permission rejection.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "consume the single-use effect permission when committing a proven failure"
    )]
    pub fn fail_running(
        &self,
        permit: RunningOperation,
        error: ErrorData,
    ) -> Result<OpRecord, OperationError> {
        let row = self.permitted(&permit)?;
        self.store_failure(row, error)
    }

    /// Retains uncertainty when an external effect could have begun.
    ///
    /// # Errors
    ///
    /// Returns typed persistence errors or a stale-permission rejection.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "consume the single-use effect permission even when its outcome is unknown"
    )]
    pub fn outcome_unknown(&self, permit: RunningOperation) -> Result<OpRecord, OperationError> {
        let mut row = self.permitted(&permit)?;
        row.entry.record.state = OpenEnum::Known(OpState::OutcomeUnknown);
        self.update(row)
    }

    /// Cancels only before intent; otherwise returns the retained receipt or pending state.
    ///
    /// # Errors
    ///
    /// Returns lookup, expiration or CAS failures; never reports an unproven rollback.
    pub fn cancel(&self, op: &OpId) -> Result<OpCancelResult, OperationError> {
        self.get(op)?;
        let mut row = self.current(op)?;
        let outcome = match &row.entry.record.state {
            OpenEnum::Known(OpState::Accepted) => {
                row.entry.record.state = OpenEnum::Known(OpState::Cancelled);
                return Ok(OpCancelResult::new(
                    self.update(row)?,
                    CancelOutcome::CancelledBeforeCommit,
                ));
            }
            OpenEnum::Known(OpState::Committed) => CancelOutcome::AlreadyCommitted,
            OpenEnum::Known(OpState::Cancelled | OpState::Failed) => {
                CancelOutcome::CancelledBeforeCommit
            }
            OpenEnum::Known(OpState::Running | OpState::OutcomeUnknown) => {
                row.entry
                    .record
                    .evidence
                    .get_or_insert_default()
                    .cancellation_requested = true;
                return Ok(OpCancelResult::new(
                    self.update(row)?,
                    CancelOutcome::Pending,
                ));
            }
            OpenEnum::Known(_) | OpenEnum::Unknown(_) => return Err(OperationError::InvalidLedger),
        };
        Ok(OpCancelResult::new(row.entry.record, outcome))
    }

    /// Accepts a request whose cancellation arrived before its durable accept.
    ///
    /// A new identity commits `op.accepted` and `op.cancelled` in one transaction, so no
    /// restart or replay can observe it as runnable. An equivalent retained identity is
    /// cancelled exactly as [`Operations::cancel`] would.
    ///
    /// # Errors
    ///
    /// Returns the acceptance errors of [`Operations::accept`] or the cancellation errors.
    pub fn accept_cancelled(&self, request: &Request) -> Result<OpCancelResult, OperationError> {
        self.cancelled(request, self.admit(request, (Admission::Cancelled, None))?)
    }

    /// [`Operations::accept_cancelled`] whose first ledger read uses a slot granted in turn.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Operations::accept_cancelled`].
    pub fn accept_cancelled_granted(
        &self,
        request: &Request,
        slot: SnapshotSlot,
    ) -> Result<OpCancelResult, OperationError> {
        self.cancelled(
            request,
            self.admit(request, (Admission::Cancelled, Some(slot)))?,
        )
    }

    fn cancelled(
        &self,
        request: &Request,
        admitted: Admitted,
    ) -> Result<OpCancelResult, OperationError> {
        match admitted {
            Admitted::New(record) => Ok(OpCancelResult::new(*record, CancelOutcome::CancelPending)),
            Admitted::Retained => self.cancel(request.op_id()),
        }
    }

    fn store_failure(
        &self,
        mut row: super::ledger::Row,
        error: ErrorData,
    ) -> Result<OpRecord, OperationError> {
        row.entry.record.state = OpenEnum::Known(OpState::Failed);
        row.entry.record.error = Some(error);
        self.update(row)
    }

    pub(super) fn permitted(
        &self,
        permit: &RunningOperation,
    ) -> Result<super::ledger::Row, OperationError> {
        if permit.context.principal != self.context.principal
            || permit.context.identity != self.context.identity
        {
            return Err(OperationError::InvalidLedger);
        }
        let row = self.current(&permit.op_id)?;
        match row.entry.record.state {
            OpenEnum::Known(OpState::Running)
                if row.entry.boot == self.context.identity.server_boot_id() =>
            {
                Ok(row)
            }
            _ => Err(OperationError::Settled(Box::new(row.entry.record))),
        }
    }
}
