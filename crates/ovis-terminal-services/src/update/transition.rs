//! `update.activate` and `update.rollback`: journaled selection swaps that only stand once
//! readiness and the control restart succeeded, and are reverted with a receipt otherwise.

use ovis_terminal_protocol::domain::update::{
    ActivationPhase, ActiveBuild, ProbeOutcome, RejectReason, RollbackReason, UpdateRejection,
};
use ovis_terminal_protocol::schema::methods::update::{
    UpdateRecord, UpdateState, UpdateStepParams,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::digest::hash_file;
use super::layout::binary_name;
use super::probe::{Expectation, probe};
use super::store::{Journal, Selected, Selection, Step};
use super::{UpdateError, Updates};

impl Updates {
    /// Selects a verified build once it proves ready and control restarted onto it.
    ///
    /// # Errors
    ///
    /// `no_staged_update`, `invalid_update_state` unless it is verified,
    /// `integrity_failed` if its executable changed, `activation_failed` with the
    /// rollback recorded, `rollback_failed`, or install-root I/O.
    pub fn activate(&self, params: &UpdateStepParams) -> Result<UpdateRecord, UpdateError> {
        let mut state = self.lock()?;
        if state.pending.is_some() {
            return Err(UpdateError::ActivationPending);
        }
        let mut record = state.record(&params.update_id)?;
        let (OpenEnum::Known(UpdateState::Verified), Some(build)) =
            (&record.state, record.build.clone())
        else {
            return Err(UpdateError::InvalidState { step: "activate" });
        };
        let binary = self
            .layout
            .build(&record.update_id)
            .join(binary_name(&record.target));
        if hash_file(&binary, self.config.limits.archive_bytes)? != build.binary_sha256 {
            record.state = OpenEnum::Known(UpdateState::Rejected);
            let rejection =
                UpdateRejection::new(RejectReason::DigestMismatch, params.op_id.clone());
            record.rejection = Some(rejection);
            self.save(&mut state, &record)?;
            return Err(UpdateError::Rejected(RejectReason::DigestMismatch));
        }
        let next = Selected {
            build: ActiveBuild::new(
                record.version.clone(),
                record.target.clone(),
                build.binary_sha256.clone(),
            ),
            binary,
            update_id: Some(record.update_id.clone()),
        };
        let before = state.selection.clone();
        let after = before.advanced(next);
        let mut journal = Journal {
            update_id: record.update_id.clone(),
            op_id: params.op_id.clone(),
            step: Step::Activate,
            phase: ActivationPhase::Intent,
            before,
            after,
            readiness: None,
            restart: None,
        };
        self.journal(&mut state, &journal)?;
        self.select(&mut state, &journal.after)?;
        journal.phase = ActivationPhase::Selected;
        self.journal(&mut state, &journal)?;
        let expect = Expectation {
            version: &record.version,
            protocols: &build.manifest.protocol_generations,
            deadline: self.config.limits.probe_deadline,
        };
        let readiness = match probe(&journal.after.active.binary, &expect) {
            Ok(readiness) => readiness,
            Err(error) => {
                self.restore(
                    &mut state,
                    (&journal, RollbackReason::ReadinessFailed, None),
                )?;
                return Err(error);
            }
        };
        if readiness.outcome != OpenEnum::Known(ProbeOutcome::Ready) {
            self.restore(
                &mut state,
                (&journal, RollbackReason::ReadinessFailed, Some(readiness)),
            )?;
            return Err(UpdateError::ActivationFailed(
                RollbackReason::ReadinessFailed,
            ));
        }
        match self.control.restart(&params.op_id) {
            Ok(restart) => journal.restart = Some(restart),
            Err(_refused) => {
                self.restore(
                    &mut state,
                    (&journal, RollbackReason::RestartFailed, Some(readiness)),
                )?;
                return Err(UpdateError::ActivationFailed(RollbackReason::RestartFailed));
            }
        }
        journal.readiness = Some(readiness);
        journal.phase = ActivationPhase::Ready;
        self.journal(&mut state, &journal)?;
        let record = self.finish(&mut state, &journal);
        drop(state);
        record
    }

    /// Selects the build the active update replaced and restarts control onto it.
    ///
    /// # Errors
    ///
    /// `no_staged_update`, `invalid_update_state` unless it is the active update,
    /// `rollback_failed` with the selection unchanged, or install-root I/O.
    pub fn rollback(&self, params: &UpdateStepParams) -> Result<UpdateRecord, UpdateError> {
        let mut state = self.lock()?;
        if state.pending.is_some() {
            return Err(UpdateError::ActivationPending);
        }
        let record = state.record(&params.update_id)?;
        let selection = state.selection.clone();
        let current = selection.active.update_id.as_ref() == Some(&record.update_id);
        let (true, Some(previous)) = (current, selection.previous.clone()) else {
            return Err(UpdateError::InvalidState { step: "rollback" });
        };
        let digest =
            hash_file(&previous.binary, self.config.limits.archive_bytes).map_err(|error| {
                UpdateError::RollbackFailed {
                    source: Box::new(error),
                }
            })?;
        if digest != previous.build.binary_sha256 {
            let changed = UpdateError::Rejected(RejectReason::DigestMismatch);
            return Err(UpdateError::RollbackFailed {
                source: Box::new(changed),
            });
        }
        let after = Selection {
            active: previous,
            previous: Some(selection.active.clone()),
        };
        let mut journal = Journal {
            update_id: record.update_id,
            op_id: params.op_id.clone(),
            step: Step::Rollback,
            phase: ActivationPhase::Intent,
            before: selection,
            after,
            readiness: None,
            restart: None,
        };
        self.journal(&mut state, &journal)?;
        self.select(&mut state, &journal.after)?;
        journal.phase = ActivationPhase::Selected;
        self.journal(&mut state, &journal)?;
        match self.control.restart(&params.op_id) {
            Ok(restart) => journal.restart = Some(restart),
            Err(refused) => {
                self.restore(&mut state, (&journal, RollbackReason::RestartFailed, None))?;
                return Err(UpdateError::RollbackFailed {
                    source: Box::new(refused),
                });
            }
        }
        journal.phase = ActivationPhase::Ready;
        self.journal(&mut state, &journal)?;
        let record = self.finish(&mut state, &journal);
        drop(state);
        record
    }
}
