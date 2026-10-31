//! Reconcile actual holder ownership before publishing a control restart.

use ovis_terminal_holder::protocol::{HeldTerminal, Message, TerminalTarget, Welcome};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Revision};
use ovis_terminal_store::{CasWrite, OpId, Payload, Version};

use super::super::Terminals;
use crate::events::{Commit, ResourceId};
use crate::ops::OperationError;
use crate::registry::{
    IdentityReservation, LifecycleEvent as Transition, ProcessGeneration, ProcessHandle, ProcessId,
    ProcessIdentity, ProcessPid, ProcessStartToken, RelaunchEvidence, ReservationError,
    SurvivalEvidence, TerminalGeneration, TerminalId, TerminalLifecycle, TerminalState,
};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{LaunchState, LifecycleEvent, Record, TerminalError};

impl Terminals {
    pub(super) fn adopted(&mut self, welcome: Welcome) -> Result<(), TerminalError> {
        let mut adopted = std::collections::BTreeSet::new();
        for held in welcome.terminals {
            let id: TerminalId = held.terminal_id.as_str().parse()?;
            adopted.insert(id);
            let mut record = self.record(id)?;
            if matches!(record.spawn, LaunchState::Pending | LaunchState::Admitted) {
                record = self.adopt_pending(&held)?;
            }
            let matches = record.lifecycle.last_process().is_some_and(|process| {
                held.pid
                    .is_some_and(|pid| u64::from(pid) == process.pid().get())
                    && held
                        .start_token
                        .as_ref()
                        .is_some_and(|token| token.as_str() == process.start_token().as_str())
                    && record.lifecycle.generation().get() == held.terminal_incarnation.get()
            });
            if !matches {
                return Err(TerminalError::ProcessReplaced);
            }
            record.output_offset = ByteOffset::new(held.output_offset);
            record.control = crate::terminals::ControlState::Running;
            self.persist(&record, LifecycleEvent::Updated(record.clone()))?;
            if let Some(exit) = held.exit {
                self.observe(Message::Exit(exit))?;
            }
            let mut record = self.record(id)?;
            if record.lifecycle.identity().server_boot_id() != self.host.identity.server_boot_id() {
                let survival = record
                    .lifecycle
                    .process()
                    .map_or(SurvivalEvidence::NoLiveProcess, |process| {
                        SurvivalEvidence::HolderConfirmed(process.clone())
                    });
                record.lifecycle = record.lifecycle.transition(Transition::CoreRestart {
                    server_boot_id: self.host.identity.server_boot_id(),
                    survival,
                })?;
                self.persist(&record, LifecycleEvent::Updated(record.clone()))?;
            }
        }
        for mut record in self.core.read_snapshot()?.sections.terminals {
            if !adopted.contains(&record.id()) {
                if record.lifecycle.state() == TerminalState::Running {
                    record.lifecycle = record.lifecycle.transition(Transition::HolderLost)?;
                    record.agent_owner = None;
                }
                if record.lifecycle.identity().server_boot_id()
                    != self.host.identity.server_boot_id()
                {
                    record.lifecycle = record.lifecycle.transition(Transition::CoreRestart {
                        server_boot_id: self.host.identity.server_boot_id(),
                        survival: SurvivalEvidence::NoLiveProcess,
                    })?;
                }
                record.control = crate::terminals::ControlState::Running;
                self.persist(&record, LifecycleEvent::Updated(record.clone()))?;
            }
        }
        Ok(())
    }

    fn adopt_pending(&self, held: &HeldTerminal) -> Result<Record, TerminalError> {
        let mut adopted = None;
        let cursor = self.core.commit_unit(|snapshot| {
            let (commit, record) = self.stage_adoption(snapshot, held)?;
            adopted = Some(record);
            Ok::<_, OperationError>(commit)
        })?;
        let mut record = adopted.ok_or(TerminalError::NotFound)?;
        record.revision = Revision::new(cursor.seq.get());
        Ok(record)
    }

    fn stage_adoption(
        &self,
        snapshot: &ovis_terminal_store::Snapshot,
        held: &HeldTerminal,
    ) -> Result<(Commit<LifecycleEvent>, Record), OperationError> {
        let id: TerminalId = held
            .terminal_id
            .as_str()
            .parse()
            .map_err(ReservationError::from)?;
        let (mut record, version) = read_record(snapshot, id)?;
        let target = record
            .holder_target
            .as_ref()
            .ok_or(OperationError::EvidenceMismatch)?;
        if target.terminal_incarnation != held.terminal_incarnation {
            return Err(OperationError::EvidenceMismatch);
        }
        let (process_id, writes) = IdentityReservation::stage::<ProcessId>(snapshot)?.into_parts();
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
                ProcessPid::try_from(u64::from(held.pid.ok_or(OperationError::EvidenceMismatch)?))
                    .map_err(ReservationError::from)?,
                ProcessStartToken::new(
                    held.start_token
                        .as_ref()
                        .ok_or(OperationError::EvidenceMismatch)?
                        .as_str(),
                )
                .map_err(ReservationError::from)?,
            ),
        );
        let transition = if record.lifecycle.last_process().is_some() {
            Transition::Relaunch {
                evidence: RelaunchEvidence::SpawnConfirmed,
                process: process.clone(),
            }
        } else {
            record.lifecycle = TerminalLifecycle::new(
                self.host.identity,
                id,
                TerminalGeneration::new(held.terminal_incarnation.get())
                    .map_err(ReservationError::from)?,
            );
            Transition::InitialSpawn {
                evidence: RelaunchEvidence::SpawnConfirmed,
                process: process.clone(),
            }
        };
        record.lifecycle = record
            .lifecycle
            .transition(transition)
            .map_err(|_| OperationError::EvidenceMismatch)?;
        if record.lifecycle.generation().get() != held.terminal_incarnation.get() {
            return Err(OperationError::EvidenceMismatch);
        }
        if record.lifecycle.identity().server_boot_id() != self.host.identity.server_boot_id() {
            record.lifecycle = record
                .lifecycle
                .transition(Transition::CoreRestart {
                    server_boot_id: self.host.identity.server_boot_id(),
                    survival: SurvivalEvidence::HolderConfirmed(process),
                })
                .map_err(|_| OperationError::EvidenceMismatch)?;
        }
        record.holder_target = Some(TerminalTarget {
            terminal_id: held.terminal_id.clone(),
            terminal_incarnation: held.terminal_incarnation,
            process_incarnation: Some(
                held.process_incarnation
                    .ok_or(OperationError::EvidenceMismatch)?,
            ),
        });
        record.spawn = LaunchState::Spawned;
        record.output_offset = ByteOffset::new(held.output_offset);
        record.control = crate::terminals::ControlState::Running;
        record.agent_owner = None;
        let mut commit = Commit::new(
            OpId::new(uuid::Uuid::now_v7().to_string()),
            LifecycleEvent::Spawned(record.clone()),
            vec![ResourceId::Terminal(id)],
        )
        .write(CasWrite::new(
            record_key(id),
            Version::new(version.get()),
            Some(Payload::from(serde_json::to_vec(&record)?)),
        ));
        for write in writes {
            commit = commit.write(write);
        }
        Ok((commit, record))
    }
}
