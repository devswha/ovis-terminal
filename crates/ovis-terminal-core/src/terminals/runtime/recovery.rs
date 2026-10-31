//! Explicit relaunch preserves the terminal ID, never an old process incarnation.

use ovis_terminal_authz::{AccessRequest, Action};
use ovis_terminal_holder::protocol::{Forget, TerminalIncarnation, TerminalTarget};
use ovis_terminal_protocol::schema::methods::server::{OpState, OperationEvidence};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Incarnation, OpenEnum};
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::{Call, Intent, Pending, Terminals, now};
use crate::events::ResourceId;
use crate::ops::{Completion, Context, Request};
use crate::registry::{TerminalId, TerminalState};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{LaunchState, LifecycleEvent, Record, TerminalError};

impl Terminals {
    /// Relaunches a kept terminal only after the previous process is proven ended.
    ///
    /// # Errors
    ///
    /// Returns `not_lost` rather than duplicating an uncertain or still-running process,
    /// or typed authority, holder and persistence failures.
    pub fn recover(&mut self, call: Call<'_>, id: TerminalId) -> Result<Record, TerminalError> {
        self.ensure_running()?;
        let mut record = self.record(id)?;
        let decision = self.authorize(call.session, (&record, Action::Launch))?;
        let operations = self.operations(call.session)?;
        let request = Request::new(
            call.op_id,
            "terminal.recover".into(),
            serde_json::json!({"terminal":id}),
        )?;
        let accepted = operations.accept(&request)?;
        if accepted.state != OpenEnum::Known(OpState::Accepted) {
            return self.record(id);
        }
        let ended = match record.lifecycle.state() {
            TerminalState::Finished => true,
            TerminalState::Running => false,
            TerminalState::RestoredMetadata => {
                record.spawn == LaunchState::Failed
                    || record.origin
                        == ovis_terminal_protocol::schema::records::TerminalOrigin::RestoredMetadata
            }
            TerminalState::Lost => {
                let previous = record
                    .lifecycle
                    .last_process()
                    .ok_or(TerminalError::ProcessReplaced)?;
                let pid = ovis_terminal_holder::process::ProcessId::new(
                    u32::try_from(previous.pid().get())
                        .map_err(|_| TerminalError::ProcessReplaced)?,
                );
                match ovis_terminal_holder::process::read_start_token(pid) {
                    Ok(current) => current.as_str() != previous.start_token().as_str(),
                    Err(ovis_terminal_holder::process::StartTokenError::ProcessGone { .. }) => true,
                    Err(error) => return Err(error.into()),
                }
            }
        };
        if !record.launch.keep
            || matches!(record.spawn, LaunchState::Admitted | LaunchState::Pending)
            || !ended
        {
            return Err(TerminalError::NotLost);
        }
        let permit = operations.start(request.op_id())?;
        let generation = record.lifecycle.generation().checked_next()?;
        let previous_exit = record.exit.take();
        record.launch_op = request.op_id().clone();
        record.holder_target = Some(TerminalTarget {
            terminal_id: ovis_terminal_holder::protocol::TerminalId::new(id.to_string()),
            terminal_incarnation: TerminalIncarnation::new(generation.get()),
            process_incarnation: None,
        });
        record.spawn = LaunchState::Pending;
        record.hangup = None;
        record.failure = None;
        record.rejection = None;
        record.agent_owner = None;
        record.output_offset = ByteOffset::new(0);
        record.origin = ovis_terminal_protocol::schema::records::TerminalOrigin::Relaunched;
        operations.record_unit(&permit, |snapshot| {
            self.host.authority.recheck(
                &decision,
                &AccessRequest::new(Action::Launch, decision.target().clone(), now()?),
            )?;
            let (_, version) = read_record(snapshot, id)?;
            super::stamp_operation(&mut record, snapshot)?;
            let evidence = OperationEvidence::for_process(
                ovis_terminal_protocol::schema::ids::TerminalId::new(id.to_string()),
                (Incarnation::new(generation.get()), None),
            );
            Ok(Completion::pending()
                .evidence(evidence)
                .write(CasWrite::new(
                    record_key(id),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(&record)?)),
                ))
                .event(
                    LifecycleEvent::Pending(record.clone()),
                    vec![ResourceId::Terminal(id)],
                ))
        })?;
        self.pending.insert(
            id,
            Pending {
                permit,
                context: Context::new(call.session.principal().clone(), self.host.identity, now()?),
                intent: Intent::Launch,
            },
        );
        if let Some(exit) = previous_exit {
            self.host.holder.forget(&Forget {
                target: exit.target,
                acknowledged_final_offset: Some(exit.final_offset),
            })?;
        }
        if let Err(error) = self.start_worker(&record, decision) {
            self.rejected(id, error)?;
        }
        self.record(id)
    }
}
