//! Administrative verbs make their process scope explicit and inspectable.

use ovis_terminal_authz::{AccessRequest, Action};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::{Call, Terminals, now};
use crate::events::ResourceId;
use crate::ops::{Completion, Request};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{
    ControlReceipt, ControlState, KillRequest, LaunchState, LifecycleEvent, TerminalError,
};

impl Terminals {
    /// Stops control ownership without terminating any held program.
    ///
    /// # Errors
    ///
    /// Returns authority, deduplication or durable-commit failures.
    pub fn control_stop(&mut self, call: Call<'_>) -> Result<ControlReceipt, TerminalError> {
        self.park(call, ControlState::Stopped)
    }

    /// Releases ownership for a replacement core without claiming that it already booted.
    ///
    /// # Errors
    ///
    /// Returns authority, deduplication or durable-commit failures.
    pub fn prepare_restart(&mut self, call: Call<'_>) -> Result<ControlReceipt, TerminalError> {
        self.park(call, ControlState::Handover)
    }

    fn park(
        &mut self,
        call: Call<'_>,
        state: ControlState,
    ) -> Result<ControlReceipt, TerminalError> {
        let decision = self.host.authority.authorize(
            call.session,
            &AccessRequest::new(Action::Administer, self.machine_target(), now()?),
        )?;
        let method = match state {
            ControlState::Stopped => "server.stop_control",
            ControlState::Handover => "server.prepare_restart",
            ControlState::Running => return Err(TerminalError::InvalidParams),
        };
        let operations = self.operations(call.session)?;
        let request = Request::new(call.op_id, method.into(), serde_json::json!({}))?;
        let accepted = operations.accept(&request)?;
        if accepted.state == OpenEnum::Known(OpState::Committed) {
            return Ok(serde_json::from_value(
                accepted.result.ok_or(TerminalError::NotFound)?,
            )?);
        }
        self.ensure_running()?;
        let permit = operations.start(request.op_id())?;
        let mut records = self.core.read_snapshot()?.sections.terminals;
        let preserved = records
            .iter()
            .filter_map(|record| record.lifecycle.process().cloned())
            .collect();
        let pending = records
            .iter()
            .filter(|record| matches!(record.spawn, LaunchState::Admitted | LaunchState::Pending))
            .map(crate::terminals::Record::id)
            .collect();
        let receipt = match state {
            ControlState::Stopped => ControlReceipt::Stopped { preserved, pending },
            ControlState::Handover => ControlReceipt::HandoverPrepared { preserved, pending },
            ControlState::Running => return Err(TerminalError::InvalidParams),
        };
        operations.commit_unit(permit, |snapshot| {
            self.host.authority.recheck(
                &decision,
                &AccessRequest::new(Action::Administer, self.machine_target(), now()?),
            )?;
            let mut completion = Completion::new(serde_json::to_value(&receipt)?);
            let mut affected = vec![ResourceId::Machine(self.host.identity.machine_id())];
            for record in &mut records {
                record.control = state;
                let (_, version) = read_record(snapshot, record.id())?;
                super::stamp_operation(record, snapshot)?;
                completion = completion.write(CasWrite::new(
                    record_key(record.id()),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(record)?)),
                ));
                affected.push(ResourceId::Terminal(record.id()));
            }
            Ok(completion.event(LifecycleEvent::Control(receipt.clone()), affected))
        })?;
        self.control = state;
        self.link.release();
        Ok(receipt)
    }

    /// Explicitly stops all currently owned programs while leaving control available.
    ///
    /// # Errors
    ///
    /// Returns authority, process or persistence errors. An interrupted operation is
    /// never automatically repeated.
    pub fn program_stop(&mut self, call: Call<'_>) -> Result<ControlReceipt, TerminalError> {
        self.ensure_running()?;
        let decision = self.host.authority.authorize(
            call.session,
            &AccessRequest::new(Action::Administer, self.machine_target(), now()?),
        )?;
        let request = Request::new(
            call.op_id,
            "server.stop_programs".into(),
            serde_json::json!({}),
        )?;
        let accepted = self.operations(call.session)?.accept(&request)?;
        if accepted.state == OpenEnum::Known(OpState::Committed) {
            return Ok(serde_json::from_value(
                accepted.result.ok_or(TerminalError::NotFound)?,
            )?);
        }
        let permit = self.operations(call.session)?.start(request.op_id())?;
        let records = self.core.read_snapshot()?.sections.terminals;
        let mut ended = Vec::new();
        let mut unknown = Vec::new();
        let mut pending = Vec::new();
        for mut record in records {
            if matches!(record.spawn, LaunchState::Admitted | LaunchState::Pending) {
                match self.await_spawn(record.id(), std::time::Duration::from_secs(10)) {
                    Ok(settled) => record = settled,
                    Err(TerminalError::Notification(
                        std::sync::mpsc::RecvTimeoutError::Timeout,
                    )) => {
                        pending.push(record.id());
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            if let Some(process) = record.lifecycle.process() {
                self.host.authority.recheck(
                    &decision,
                    &AccessRequest::new(Action::Administer, self.machine_target(), now()?),
                )?;
                let outcome = self.kill(
                    Call::new(call.session, OpId::new(uuid::Uuid::now_v7().to_string())),
                    &KillRequest::immediate(record.id(), process.clone()),
                )?;
                if outcome.exit.as_ref().is_some_and(|exit| {
                    exit.exit_unknown.is_none() && (exit.code.is_some() || exit.signal.is_some())
                }) {
                    ended.push(process.clone());
                } else {
                    unknown.push(process.clone());
                }
            } else if record.lifecycle.state() == crate::registry::TerminalState::Lost
                && let Some(previous) = record.lifecycle.last_process()
            {
                unknown.push(previous.clone());
            }
        }
        let receipt = ControlReceipt::ProgramStop {
            ended,
            unknown,
            pending,
        };
        self.operations(call.session)?
            .commit_unit(permit, |_snapshot| {
                self.host.authority.recheck(
                    &decision,
                    &AccessRequest::new(Action::Administer, self.machine_target(), now()?),
                )?;
                Ok(Completion::new(serde_json::to_value(&receipt)?).event(
                    LifecycleEvent::Control(receipt.clone()),
                    vec![ResourceId::Machine(self.host.identity.machine_id())],
                ))
            })?;
        Ok(receipt)
    }
}
