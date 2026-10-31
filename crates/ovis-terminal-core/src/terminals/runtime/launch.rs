//! Durable launch admission precedes the bounded holder worker's external effect.

use ovis_terminal_authz::{AccessRequest, Action};
use ovis_terminal_holder::protocol::{Spawn, TerminalIncarnation, TerminalTarget};
use ovis_terminal_holder::unix::HolderError;
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::methods::server::{OpState, OperationEvidence};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Incarnation, OpenEnum, Revision};
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::{Call, Incoming, Intent, Pending, Terminals, now};
use crate::ops::{Completion, Context, OperationError, Request};
use crate::registry::{IdentityReservation, TerminalGeneration, TerminalId, TerminalLifecycle};
use crate::terminals::snapshot::{index_key, read_index, record_key};
use crate::terminals::{Launch, LaunchState, LifecycleEvent, Record, TerminalError};

#[derive(Debug)]
struct Admission {
    record: Record,
    effect: Option<(
        ovis_terminal_authz::Decision<crate::registry::RegistryIdentities>,
        Pending,
    )>,
}

impl Terminals {
    /// Admits one terminal and starts its holder worker without waiting for process output.
    ///
    /// # Errors
    ///
    /// Returns validation, authority, duplicate-name, operation or durable-commit failures.
    /// A repeated operation never starts a second worker.
    pub fn run(&mut self, call: Call<'_>, launch: Launch) -> Result<Record, TerminalError> {
        let Admission { mut record, effect } = self.admit_launch(call, launch)?;
        let Some((decision, pending)) = effect else {
            return Ok(record);
        };
        record.spawn = LaunchState::Pending;
        record.holder_target = Some(TerminalTarget {
            terminal_id: ovis_terminal_holder::protocol::TerminalId::new(record.id().to_string()),
            terminal_incarnation: TerminalIncarnation::new(record.lifecycle.generation().get()),
            process_incarnation: None,
        });
        self.persist(&record, LifecycleEvent::Pending(record.clone()))?;
        self.pending.insert(record.id(), pending);
        if let Err(error) = self.start_worker(&record, decision) {
            self.rejected(record.id(), error)?;
        }
        self.record(record.id())
    }

    fn admit_launch(&self, call: Call<'_>, mut launch: Launch) -> Result<Admission, TerminalError> {
        self.ensure_running()?;
        launch.validate()?;
        launch.program.cwd =
            std::fs::canonicalize(&launch.program.cwd).map_err(TerminalError::Cwd)?;
        let params = serde_json::to_value(&launch)?;
        let decision = self.host.authority.authorize(
            call.session,
            &AccessRequest::new(Action::Launch, self.machine_target(), now()?),
        )?;
        let operations = self.operations(call.session)?;
        let request = Request::new(call.op_id, "terminal.run".into(), params)?;
        let accepted = operations.accept(&request)?;
        if accepted.state != OpenEnum::Known(OpState::Accepted) {
            let id = accepted
                .evidence
                .as_ref()
                .and_then(|proof| proof.terminal_id.as_ref())
                .ok_or(OperationError::Settled(Box::new(accepted.clone())))?;
            return Ok(Admission {
                record: self.record(id.as_str().parse()?)?,
                effect: None,
            });
        }
        self.check_launch_name(&launch)?;
        let permit = operations.start(request.op_id())?;
        let mut created = None;
        let identity = self.host.identity;
        operations.record_unit(&permit, |snapshot| {
            self.host.authority.recheck(
                &decision,
                &AccessRequest::new(Action::Launch, self.machine_target(), now()?),
            )?;
            let (id, writes) = IdentityReservation::stage::<TerminalId>(snapshot)?.into_parts();
            let (mut index, version) = read_index(snapshot, identity.runtime_id())?;
            if index.len() >= 1024 {
                return Err(OperationError::Capacity);
            }
            let mut record = Record {
                control: crate::terminals::ControlState::Running,
                launch_op: request.op_id().clone(),
                lifecycle: TerminalLifecycle::new(
                    identity,
                    id,
                    TerminalGeneration::new(1).map_err(crate::registry::ReservationError::from)?,
                ),
                launch,
                spawn: LaunchState::Admitted,
                holder_target: None,
                hangup: None,
                exit: None,
                failure: None,
                rejection: None,
                agent_owner: None,
                output_offset: ByteOffset::new(0),
                revision: Revision::new(0),
                origin: ovis_terminal_protocol::schema::records::TerminalOrigin::Live,
            };
            super::stamp_operation(&mut record, snapshot)?;
            index.push(id);
            let evidence = OperationEvidence::for_process(
                WireTerminalId::new(id.to_string()),
                (Incarnation::new(1), None),
            );
            let mut completion = Completion::pending()
                .evidence(evidence)
                .write(CasWrite::new(
                    index_key(identity.runtime_id()),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(&index)?)),
                ))
                .write(CasWrite::new(
                    record_key(id),
                    Version::ABSENT,
                    Some(Payload::from(serde_json::to_vec(&record)?)),
                ))
                .event(
                    LifecycleEvent::Admitted(record.clone()),
                    vec![crate::events::ResourceId::Terminal(id)],
                );
            for write in writes {
                completion = completion.write(write);
            }
            created = Some(record);
            Ok(completion)
        })?;
        let record = created.ok_or(TerminalError::NotFound)?;
        let context = Context::new(call.session.principal().clone(), identity, now()?);
        Ok(Admission {
            record,
            effect: Some((
                decision,
                Pending {
                    permit,
                    context,
                    intent: Intent::Launch,
                },
            )),
        })
    }

    fn check_launch_name(&self, launch: &Launch) -> Result<(), TerminalError> {
        if let Some(name) = &launch.name
            && self
                .core
                .read_snapshot()?
                .sections
                .terminals
                .iter()
                .any(|record| {
                    record.launch.name.as_ref() == Some(name)
                        && record.exit.is_none()
                        && record.spawn != LaunchState::Failed
                })
        {
            return Err(TerminalError::DuplicateName);
        }
        Ok(())
    }

    pub(super) fn start_worker(
        &mut self,
        record: &Record,
        decision: ovis_terminal_authz::Decision<crate::registry::RegistryIdentities>,
    ) -> Result<(), TerminalError> {
        if self.workers.len() >= 1024 {
            return Err(TerminalError::InvalidParams);
        }
        let spawn = Spawn {
            terminal_id: ovis_terminal_holder::protocol::TerminalId::new(record.id().to_string()),
            terminal_incarnation: record
                .holder_target
                .as_ref()
                .ok_or(TerminalError::InvalidParams)?
                .terminal_incarnation,
            argv: record.launch.program.argv.clone(),
            cwd: record.launch.program.cwd.clone(),
            env: record.launch.program.env.clone(),
            geometry: (record.launch.geometry.cols, record.launch.geometry.rows),
        };
        let id = record.id();
        let holder = self.host.holder.clone();
        let authority = std::sync::Arc::clone(&self.host.authority);
        let sender = self.sender.clone();
        let target = decision.target().clone();
        let worker = std::thread::Builder::new()
            .name("ovis-terminal-launch".into())
            .spawn(move || {
                let result: Result<_, TerminalError> =
                    now().map_err(TerminalError::from).and_then(|at| {
                        authority
                            .recheck(&decision, &AccessRequest::new(Action::Launch, target, at))?;
                        Ok(holder.spawn_wire(&spawn)?)
                    });
                match result {
                    Ok(_)
                    | Err(TerminalError::Holder(
                        HolderError::Launch { .. } | HolderError::Draining,
                    )) => {}
                    Err(error) => match sender.send(Incoming::Rejected(id, error)) {
                        Ok(()) | Err(_) => {}
                    },
                }
            })
            .map_err(TerminalError::Worker)?;
        self.workers.push(worker);
        Ok(())
    }
}
