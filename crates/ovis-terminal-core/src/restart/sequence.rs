//! The planned restart sequence and the recovery a successor finishes it with.

use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_protocol::domain::restart::{BoundaryPhase, RestartBoundary, RestartPhase};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId as WireBootId};
use ovis_terminal_protocol::schema::methods::server::RecoveryEvidence;
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};

use super::control::{Control, Inner};
use super::{IntentFile, Preserved, RestartError, RestartIntent, RestartReceipt};
use crate::registry::{HostIdentity, IdentityError, RuntimeIdentity, ServerBootId};
use crate::terminals::{Call, LaunchState};

/// The identity a successor serves under: the lifetime and boot the intent recorded.
pub(super) fn successor(
    intent: &RestartIntent,
    current: RuntimeIdentity,
) -> Result<RuntimeIdentity, RestartError> {
    if intent.host_boot != current.host_boot_id() {
        return Err(RestartError::HostRebooted {
            recorded: intent.host_boot,
            current: current.host_boot_id(),
        });
    }
    if intent.runtime != current.runtime_id() {
        return Err(RestartError::LifetimeMismatch {
            recorded: intent.runtime,
            current: current.runtime_id(),
        });
    }
    let host = HostIdentity::new(current.machine_id(), intent.host_boot);
    Ok(RuntimeIdentity::new(host, intent.runtime, intent.to))
}

fn boundary(intent: &RestartIntent, phase: BoundaryPhase) -> RestartBoundary {
    RestartBoundary::new(
        phase,
        RuntimeLifetimeId::new(intent.runtime.to_string()),
        (
            WireBootId::new(intent.from.to_string()),
            WireBootId::new(intent.to.to_string()),
        ),
    )
}

fn advance(file: &IntentFile, intent: &mut RestartIntent) -> Result<(), RestartError> {
    intent.phase = intent.phase.next();
    file.store(intent)
}

impl Control {
    /// Replaces the control server while every held program keeps running.
    ///
    /// # Errors
    ///
    /// `restarting` when one already runs. A failure before handover restores readiness
    /// under the old boot and removes the intent; a failure after it leaves the intent on
    /// disk for the next [`Control::open`] to finish.
    pub fn restart(&self, op_id: OpId) -> Result<RestartReceipt, RestartError> {
        self.ensure_ready()?;
        let mut inner = self.lock()?;
        self.ensure_ready()?;
        let mut intent = inner.intent(op_id)?;
        let file = inner.plan.intent();
        file.store(&intent)?;
        self.set(intent.phase)?;
        #[cfg(test)]
        self.crash_point(intent.phase)?;
        advance(&file, &mut intent)?;
        self.set(intent.phase)?;
        #[cfg(test)]
        self.crash_point(intent.phase)?;
        intent.waits = self.waits.interrupt();
        self.announce(&boundary(&intent, BoundaryPhase::Started))?;
        advance(&file, &mut intent)?;
        self.set(intent.phase)?;
        #[cfg(test)]
        self.crash_point(intent.phase)?;
        if let Err(error) = inner.hand_over() {
            file.clear()?;
            self.set(RestartPhase::Running)?;
            let mut restored = boundary(&intent, BoundaryPhase::Completed);
            restored.server_boot_id_new = restored.server_boot_id_old.clone();
            self.announce(&restored)?;
            return Err(error);
        }
        advance(&file, &mut intent)?;
        self.set(intent.phase)?;
        inner.release()?;
        #[cfg(test)]
        self.crash_point(intent.phase)?;
        drop(inner);
        self.finish(&intent, false)
    }

    /// Adopts the holder under the intent's boot, verifies and classifies, then resumes.
    pub(super) fn finish(
        &self,
        intent: &RestartIntent,
        resumed: bool,
    ) -> Result<RestartReceipt, RestartError> {
        let mut intent = intent.clone();
        let mut inner = self.lock()?;
        let file = inner.plan.intent();
        if inner.terminals.is_none() {
            let identity = successor(&intent, inner.identity)?;
            inner.terminals = Some(inner.plan.boot(identity)?);
            inner.identity = identity;
        }
        intent.phase = RestartPhase::Booted;
        file.store(&intent)?;
        self.set(intent.phase)?;
        #[cfg(test)]
        self.crash_point(intent.phase)?;
        let mut receipt = inner.recover(&intent)?;
        receipt.resumed = resumed;
        advance(&file, &mut intent)?;
        file.clear()?;
        self.set(RestartPhase::Running)?;
        drop(inner);
        let mut completed = boundary(&intent, BoundaryPhase::Completed);
        completed.evidence = Some(RecoveryEvidence::restored(
            completed.server_boot_id_new.clone(),
            receipt.transition().preserved_incarnations,
            now()?,
        ));
        self.announce(&completed)?;
        Ok(receipt)
    }
}

impl Inner {
    pub(super) fn intent(&self, op_id: OpId) -> Result<RestartIntent, RestartError> {
        let terminals = self
            .terminals
            .as_ref()
            .ok_or(RestartError::ControlStopped)?;
        let snapshot = terminals.coordinator().read_snapshot()?;
        let from = self.identity.server_boot_id();
        let to = ServerBootId::try_from(
            from.get()
                .checked_add(1)
                .ok_or(IdentityError::CounterOverflow)?,
        )?;
        let mut intent = RestartIntent::new(
            op_id,
            (self.identity.host_boot_id(), self.identity.runtime_id()),
            (from, to),
        );
        intent.seq = EventSeq::new(snapshot.cursor.seq.get());
        for record in snapshot.sections.terminals {
            if let Some(process) = record.lifecycle.process() {
                let generation = record.lifecycle.generation();
                intent
                    .preserved
                    .push(Preserved::new(record.id(), generation, process.clone()));
            }
            if matches!(record.spawn, LaunchState::Admitted | LaunchState::Pending) {
                intent.in_flight.push(record.launch_op.clone());
            }
        }
        Ok(intent)
    }

    /// Releases holder ownership durably; no program is signalled.
    pub(super) fn hand_over(&mut self) -> Result<(), RestartError> {
        let terminals = self
            .terminals
            .as_mut()
            .ok_or(RestartError::ControlStopped)?;
        let call = Call::new(
            self.plan.session(),
            OpId::new(uuid::Uuid::now_v7().to_string()),
        );
        terminals.prepare_restart(call)?;
        Ok(())
    }

    fn release(&mut self) -> Result<(), RestartError> {
        if let Some(terminals) = self.terminals.take() {
            terminals.close()?;
        }
        Ok(())
    }
}

fn now() -> Result<TimestampMs, RestartError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(RestartError::Clock)?
        .as_millis();
    Ok(TimestampMs::new(
        u64::try_from(millis).map_err(RestartError::ClockRange)?,
    ))
}
