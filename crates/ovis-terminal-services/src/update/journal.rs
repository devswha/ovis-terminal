//! The activation journal's durable steps: select, record, close, revert and finish.
//!
//! Recovery reads only these files, so a crash between any two steps leaves a journal
//! that the next open finishes (phase `ready`) or reverts (any earlier phase).

use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_protocol::domain::update::{
    ActivationPhase, ActivationReceipt, ReadinessEvidence, RollbackReason, RollbackReceipt,
};
use ovis_terminal_protocol::schema::methods::update::{UpdateRecord, UpdateState};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TimestampMs};

use super::layout::{remove, store};
use super::service::State;
use super::store::{Journal, Selection, Step};
use super::{UpdateError, Updates};

pub(super) fn now() -> Result<TimestampMs, UpdateError> {
    let clock = |source| UpdateError::Io {
        operation: "read the clock",
        source,
    };
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| clock(std::io::Error::other(error)))?;
    let millis =
        u64::try_from(elapsed.as_millis()).map_err(|error| clock(std::io::Error::other(error)))?;
    Ok(TimestampMs::new(millis))
}

impl Updates {
    pub(super) fn journal(&self, state: &mut State, journal: &Journal) -> Result<(), UpdateError> {
        store(&self.layout.journal(), journal)?;
        state.pending = Some(journal.clone());
        Ok(())
    }

    pub(super) fn select(
        &self,
        state: &mut State,
        selection: &Selection,
    ) -> Result<(), UpdateError> {
        store(&self.layout.selection(), selection)?;
        state.selection = selection.clone();
        Ok(())
    }

    fn close(&self, state: &mut State) -> Result<(), UpdateError> {
        remove(&self.layout.journal())?;
        state.pending = None;
        Ok(())
    }

    /// Selects `journal.before` again and records why; an interrupted rollback only
    /// restores the activated build.
    pub(super) fn restore(
        &self,
        state: &mut State,
        (journal, reason, readiness): (&Journal, RollbackReason, Option<ReadinessEvidence>),
    ) -> Result<(), UpdateError> {
        self.select(state, &journal.before)
            .map_err(|error| UpdateError::RestoreFailed {
                source: Box::new(error),
            })?;
        if journal.step == Step::Activate {
            let mut record = state.record(&journal.update_id)?;
            let builds = (
                journal.after.active.build.clone(),
                journal.before.active.build.clone(),
                now()?,
            );
            let mut receipt = RollbackReceipt::new(journal.op_id.clone(), reason, builds);
            receipt.readiness = readiness;
            record.state = OpenEnum::Known(UpdateState::RolledBack);
            record.active_version = journal.before.active.build.version.clone();
            record.rollback = Some(receipt);
            self.save(state, &record)?;
        }
        self.close(state)
    }

    /// Records the transition `journal` proved ready and restarted.
    pub(super) fn finish(
        &self,
        state: &mut State,
        journal: &Journal,
    ) -> Result<UpdateRecord, UpdateError> {
        self.select(state, &journal.after)?;
        let mut record = state.record(&journal.update_id)?;
        let restart = journal
            .restart
            .clone()
            .ok_or(UpdateError::InvalidState { step: "finish" })?;
        let from = journal.before.active.build.clone();
        match journal.step {
            Step::Activate => {
                let readiness = journal
                    .readiness
                    .clone()
                    .ok_or(UpdateError::InvalidState { step: "finish" })?;
                let receipt = ActivationReceipt::new(
                    journal.op_id.clone(),
                    from,
                    (readiness, restart, now()?),
                );
                record.state = OpenEnum::Known(UpdateState::Activated);
                record.activation = Some(receipt);
            }
            Step::Rollback => {
                let builds = (from, journal.after.active.build.clone(), now()?);
                let mut receipt =
                    RollbackReceipt::new(journal.op_id.clone(), RollbackReason::Requested, builds);
                receipt.restart = Some(restart);
                record.state = OpenEnum::Known(UpdateState::RolledBack);
                record.rollback = Some(receipt);
            }
        }
        record.active_version = journal.after.active.build.version.clone();
        self.save(state, &record)?;
        self.close(state)?;
        Ok(record)
    }

    /// Finishes a journaled transition that reached `ready` and reverts any other.
    pub(super) fn recover(&self) -> Result<(), UpdateError> {
        let mut state = self.lock()?;
        let Some(journal) = state.pending.clone() else {
            return Ok(());
        };
        let outcome = if journal.phase == ActivationPhase::Ready {
            self.finish(&mut state, &journal).map(drop)
        } else {
            let readiness = journal.readiness.clone();
            self.restore(
                &mut state,
                (&journal, RollbackReason::Interrupted, readiness),
            )
        };
        drop(state);
        outcome
    }
}
