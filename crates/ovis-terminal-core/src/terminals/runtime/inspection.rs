//! Read-only native process sampling and process-bound agent authority.

mod proof;

use ovis_terminal_authz::{AccessRequest, Action, Session};
use ovis_terminal_holder::process::StartTokenError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::{Call, Terminals, now};
use crate::events::ResourceId;
use crate::ops::{Completion, Request};
use crate::registry::{ProcessHandle, TerminalId};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{LifecycleEvent, ProcessInfo, Record, TerminalError};

impl Terminals {
    /// Inspects only the explicit terminal, with native PID-plus-start-token evidence.
    ///
    /// # Errors
    ///
    /// Returns authority, stale identity or durable invalidation failures.
    pub fn process_info(
        &mut self,
        session: &Session,
        id: TerminalId,
    ) -> Result<ProcessInfo, TerminalError> {
        Ok(self.observe_owner(self.get(session, id)?)?.1)
    }

    fn observe_owner(&self, mut record: Record) -> Result<(Record, ProcessInfo), TerminalError> {
        let info = match proof::native_info(&record) {
            Ok(info) => info,
            Err(error) => {
                let replaced = matches!(error, TerminalError::ProcessReplaced);
                let gone = matches!(
                    error,
                    TerminalError::Process(StartTokenError::ProcessGone { .. })
                );
                if replaced {
                    record.lifecycle = record
                        .lifecycle
                        .transition(crate::registry::LifecycleEvent::HolderLost)?;
                }
                if replaced || (gone && record.agent_owner.is_some()) {
                    record.agent_owner = None;
                    self.persist(&record, LifecycleEvent::Updated(record.clone()))?;
                }
                return Err(error);
            }
        };
        if let Some(owner) = &record.agent_owner {
            match proof::includes(&info, owner) {
                Ok(false) => {
                    record.agent_owner = None;
                    self.persist(&record, LifecycleEvent::Updated(record.clone()))?;
                }
                Ok(true) | Err(TerminalError::UnsupportedField) => {}
                Err(error) => return Err(error),
            }
        }
        Ok((record, info))
    }

    /// Binds declared-agent authority to an observed launcher or child, never a brand guess.
    ///
    /// # Errors
    ///
    /// Returns `process_replaced`, unsupported evidence, authority or transaction failures.
    pub fn bind_agent(
        &mut self,
        call: Call<'_>,
        (id, process): (TerminalId, ProcessHandle),
    ) -> Result<(), TerminalError> {
        self.ensure_running()?;
        let record = self.record(id)?;
        let decision = self.authorize(call.session, (&record, Action::Report))?;
        let (mut record, info) = self.observe_owner(record)?;
        if record.lifecycle.process().is_none()
            || record.hangup.is_some()
            || !proof::includes(&info, &process)?
        {
            return Err(TerminalError::ProcessReplaced);
        }
        let operations = self.operations(call.session)?;
        let request = Request::new(
            call.op_id,
            "agent.bind_process".into(),
            serde_json::json!({"terminal":id,"process":process}),
        )?;
        let accepted = operations.accept(&request)?;
        if accepted.state
            == ovis_terminal_protocol::schema::scalar::OpenEnum::Known(
                ovis_terminal_protocol::schema::methods::server::OpState::Committed,
            )
        {
            return Ok(());
        }
        let permit = operations.start(request.op_id())?;
        let mut refused = None;
        operations.commit_unit(permit, |snapshot| {
            self.host.authority.recheck(
                &decision,
                &AccessRequest::new(Action::Report, self.target(&record), now()?),
            )?;
            if let Err(error) = proof::current(&process) {
                let failure =
                    BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false)
                        .with_hint(error.hint())
                        .with_op_id(request.op_id().clone())
                        .data()
                        .clone();
                refused = Some(error);
                return Ok(Completion::failed(failure));
            }
            let (_, version) = read_record(snapshot, id)?;
            record.agent_owner = Some(process);
            super::stamp_operation(&mut record, snapshot)?;
            Ok(Completion::new(serde_json::json!({"bound":true}))
                .write(CasWrite::new(
                    record_key(id),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(&record)?)),
                ))
                .event(
                    LifecycleEvent::Updated(record),
                    vec![ResourceId::Terminal(id)],
                ))
        })?;
        refused.map_or(Ok(()), Err)
    }

    /// Revalidates agent authority independently of a surviving shell or launcher.
    ///
    /// # Errors
    ///
    /// Returns `process_replaced` for exit/replacement, or explicit unsupported native proof.
    pub fn check_agent(
        &mut self,
        session: &Session,
        (id, expected): (TerminalId, &ProcessHandle),
    ) -> Result<(), TerminalError> {
        let record = self.record(id)?;
        self.authorize(session, (&record, Action::Report))?;
        let (record, info) = self.observe_owner(record)?;
        if record.agent_owner.as_ref() != Some(expected) || !proof::includes(&info, expected)? {
            return Err(TerminalError::ProcessReplaced);
        }
        Ok(())
    }
}
