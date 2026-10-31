//! Spawn evidence and its operation outcome commit with the terminal row.

use ovis_terminal_holder::protocol::{SpawnFailed, Spawned, TerminalTarget};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::methods::server::OperationEvidence;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::super::{Terminals, now};
use crate::events::ResourceId;
use crate::ops::{Completion, OperationError, Operations};
use crate::registry::{
    IdentityReservation, LifecycleEvent as Transition, ProcessGeneration, ProcessHandle, ProcessId,
    ProcessIdentity, ProcessPid, ProcessStartToken, RelaunchEvidence, ReservationError, TerminalId,
};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{LaunchState, LifecycleEvent, TerminalError};

impl Terminals {
    pub(super) fn spawned(&mut self, spawned: Spawned) -> Result<(), TerminalError> {
        let id: TerminalId = spawned.terminal_id.as_str().parse()?;
        let pending = self
            .pending
            .remove(&id)
            .ok_or(TerminalError::ProcessReplaced)?;
        let operations = Operations::new(&self.core, pending.context.at(now()?));
        operations.commit_unit(pending.permit, |snapshot| {
            let (mut record, version) = read_record(snapshot, id)?;
            if record.holder_target.as_ref().is_none_or(|target| {
                target.terminal_incarnation != spawned.terminal_incarnation
                    || target.terminal_id != spawned.terminal_id
            }) {
                return Err(OperationError::EvidenceMismatch);
            }
            if record.origin == ovis_terminal_protocol::schema::records::TerminalOrigin::Relaunched
                && record.lifecycle.last_process().is_none()
            {
                record.lifecycle = crate::registry::TerminalLifecycle::new(
                    self.host.identity,
                    id,
                    crate::registry::TerminalGeneration::new(spawned.terminal_incarnation.get())
                        .map_err(ReservationError::from)?,
                );
            }
            let (process_id, writes) =
                IdentityReservation::stage::<ProcessId>(snapshot)?.into_parts();
            let generation = record
                .lifecycle
                .last_process()
                .map_or_else(
                    || ProcessGeneration::new(1),
                    |previous| previous.generation().checked_next(),
                )
                .map_err(ReservationError::from)?;
            let process = ProcessIdentity::new(
                process_id,
                generation,
                ProcessHandle::new(
                    ProcessPid::try_from(u64::from(spawned.pid)).map_err(ReservationError::from)?,
                    ProcessStartToken::new(spawned.start_token.as_str())
                        .map_err(ReservationError::from)?,
                ),
            );
            let transition = if record.lifecycle.last_process().is_some() {
                Transition::Relaunch {
                    evidence: RelaunchEvidence::SpawnConfirmed,
                    process,
                }
            } else {
                Transition::InitialSpawn {
                    evidence: RelaunchEvidence::SpawnConfirmed,
                    process,
                }
            };
            record.lifecycle = record
                .lifecycle
                .transition(transition)
                .map_err(|_| OperationError::EvidenceMismatch)?;
            if record.lifecycle.generation().get() != spawned.terminal_incarnation.get() {
                return Err(OperationError::EvidenceMismatch);
            }
            record.spawn = LaunchState::Spawned;
            record.agent_owner = None;
            super::super::stamp_operation(&mut record, snapshot)?;
            record.holder_target = Some(TerminalTarget {
                terminal_id: spawned.terminal_id,
                terminal_incarnation: spawned.terminal_incarnation,
                process_incarnation: Some(spawned.process_incarnation),
            });
            let evidence = OperationEvidence::for_process(
                WireTerminalId::new(id.to_string()),
                (
                    Incarnation::new(spawned.terminal_incarnation.get()),
                    Some(Incarnation::new(spawned.process_incarnation.get())),
                ),
            );
            let mut completion = Completion::new(serde_json::to_value(&record)?)
                .evidence(evidence)
                .write(CasWrite::new(
                    record_key(id),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(&record)?)),
                ))
                .event(
                    LifecycleEvent::Spawned(record),
                    vec![ResourceId::Terminal(id)],
                );
            for write in writes {
                completion = completion.write(write);
            }
            Ok(completion)
        })?;
        Ok(())
    }

    pub(super) fn spawn_failed(&mut self, failed: SpawnFailed) -> Result<(), TerminalError> {
        let id: TerminalId = failed.terminal_id.as_str().parse()?;
        if matches!(
            failed.reason,
            ovis_terminal_holder::protocol::SpawnFailureReason::WorkerUnavailable
        ) {
            let mut record = self.record(id)?;
            record.failure = Some(failed);
            if let Some(pending) = self.pending.remove(&id) {
                Operations::new(&self.core, pending.context).outcome_unknown(pending.permit)?;
            }
            return self.persist(&record, LifecycleEvent::Updated(record.clone()));
        }
        let pending = self
            .pending
            .remove(&id)
            .ok_or(TerminalError::ProcessReplaced)?;
        let operations = Operations::new(&self.core, pending.context.at(now()?));
        operations.commit_unit(pending.permit, |snapshot| {
            let (mut record, version) = read_record(snapshot, id)?;
            record.spawn = LaunchState::Failed;
            record.failure = Some(failed);
            record.agent_owner = None;
            super::super::stamp_operation(&mut record, snapshot)?;
            let failure = BoundaryError::domain(ErrorKind::Other("spawn_failed".into()), false);
            Ok(Completion::failed(failure.data().clone())
                .write(CasWrite::new(
                    record_key(id),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(&record)?)),
                ))
                .event(
                    LifecycleEvent::SpawnFailed(record),
                    vec![ResourceId::Terminal(id)],
                ))
        })?;
        Ok(())
    }
}
