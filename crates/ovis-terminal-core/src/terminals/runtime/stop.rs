//! Staged program stopping uses full incarnation checks and actual exit evidence.

use std::time::{Duration, Instant};

use ovis_terminal_authz::{AccessRequest, Action};
use ovis_terminal_holder::protocol::{Signal, SignalKind};
use ovis_terminal_holder::unix::HolderError;
use ovis_terminal_protocol::schema::methods::server::{
    OpState, OperationEvidence, ProcessExitEvidence,
};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Incarnation, OpenEnum};
use ovis_terminal_store::{CasWrite, Payload, Version};

use super::{Call, Intent, Pending, Terminals, now};
use crate::events::ResourceId;
use crate::ops::{Completion, Context, Operations, Request};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{
    KillReceipt, KillRequest, KillStage, LifecycleEvent, Record, TerminalError,
};

impl Terminals {
    /// Delivers a staged stop and reports only the holder's observed exit facts.
    ///
    /// # Errors
    ///
    /// Returns `process_replaced` before signals for a stale PID/start-token identity,
    /// or typed authority, holder and persistence failures.
    pub fn kill(
        &mut self,
        call: Call<'_>,
        request: &KillRequest,
    ) -> Result<KillReceipt, TerminalError> {
        self.ensure_running()?;
        let record = self.record(request.terminal)?;
        let decision = self.authorize(call.session, (&record, Action::Administer))?;
        let operation = Request::new(
            call.op_id,
            "terminal.kill".into(),
            serde_json::to_value(request)?,
        )?;
        let accepted = self.operations(call.session)?.accept(&operation)?;
        if accepted.state == OpenEnum::Known(OpState::Committed) {
            return Ok(serde_json::from_value(
                accepted.result.ok_or(TerminalError::NotFound)?,
            )?);
        }
        if record.lifecycle.last_process() != Some(&request.expected) {
            return Err(TerminalError::ProcessReplaced);
        }
        if record.exit.is_some() {
            return Err(TerminalError::AlreadyFinished);
        }
        if record.lifecycle.process() != Some(&request.expected) {
            return Err(TerminalError::ProcessReplaced);
        }
        let permit = self.operations(call.session)?.start(operation.op_id())?;
        let receipt = KillReceipt {
            terminal: request.terminal,
            terminal_incarnation: record.lifecycle.generation(),
            process: request.expected.clone(),
            delivered: Vec::new(),
            exit: None,
        };
        self.pending.insert(
            request.terminal,
            Pending {
                permit,
                context: Context::new(call.session.principal().clone(), self.host.identity, now()?),
                intent: Intent::Kill(receipt),
            },
        );
        let record = self.escalate(&decision, request)?;
        if let Some(pending) = self.pending.remove(&request.terminal) {
            self.complete_kill(pending, &record)?;
        }
        let completed = self.operations(call.session)?.get(operation.op_id())?;
        Ok(serde_json::from_value(
            completed.result.ok_or(TerminalError::NotFound)?,
        )?)
    }

    fn escalate(
        &mut self,
        decision: &ovis_terminal_authz::Decision<crate::registry::RegistryIdentities>,
        request: &KillRequest,
    ) -> Result<Record, TerminalError> {
        let mut record = self.record(request.terminal)?;
        let target = record
            .holder_target
            .clone()
            .ok_or(TerminalError::AlreadyFinished)?;
        let stages = [
            (
                KillStage::Hangup,
                Duration::from_millis(request.hup_ms.get()),
            ),
            (
                KillStage::Terminate,
                Duration::from_millis(request.term_ms.get()),
            ),
            (KillStage::Kill, Duration::from_secs(10)),
        ];
        for (stage, grace) in stages {
            if record.exit.is_some() {
                break;
            }
            if record.lifecycle.process() != Some(&request.expected) {
                return Err(TerminalError::ProcessReplaced);
            }
            self.host.authority.recheck(
                decision,
                &AccessRequest::new(Action::Administer, self.target(&record), now()?),
            )?;
            let delivered = match stage {
                KillStage::Hangup => self.host.holder.hangup(&target),
                KillStage::Terminate => self.host.holder.signal(&Signal {
                    target: target.clone(),
                    signal: SignalKind::Terminate,
                }),
                KillStage::Kill => self.host.holder.signal(&Signal {
                    target: target.clone(),
                    signal: SignalKind::Kill,
                }),
            };
            match delivered {
                Ok(()) => {
                    if let Some(Pending {
                        intent: Intent::Kill(receipt),
                        ..
                    }) = self.pending.get_mut(&request.terminal)
                    {
                        receipt.delivered.push(stage);
                    }
                }
                Err(HolderError::ProcessExited) => {}
                Err(error) => return Err(error.into()),
            }
            let deadline = Instant::now()
                .checked_add(grace)
                .ok_or(TerminalError::InvalidParams)?;
            loop {
                match self.receive(deadline.saturating_duration_since(Instant::now())) {
                    Ok(()) => {}
                    Err(TerminalError::Notification(
                        std::sync::mpsc::RecvTimeoutError::Timeout,
                    )) => break,
                    Err(error) => return Err(error),
                }
                record = self.record(request.terminal)?;
                if record.exit.is_some() || Instant::now() >= deadline {
                    break;
                }
            }
            record = self.record(request.terminal)?;
        }
        Ok(record)
    }

    pub(super) fn complete_kill(
        &self,
        pending: Pending,
        record: &Record,
    ) -> Result<(), TerminalError> {
        let Intent::Kill(mut receipt) = pending.intent else {
            return Err(TerminalError::InvalidParams);
        };
        receipt.exit.clone_from(&record.exit);
        let target = record
            .holder_target
            .as_ref()
            .ok_or(TerminalError::ProcessReplaced)?;
        let mut evidence = OperationEvidence::for_process(
            ovis_terminal_protocol::schema::ids::TerminalId::new(record.id().to_string()),
            (
                Incarnation::new(target.terminal_incarnation.get()),
                target
                    .process_incarnation
                    .map(|value| Incarnation::new(value.get())),
            ),
        );
        if let Some(exit) = &record.exit {
            let mut status = ProcessExitEvidence::new(
                (exit.code, exit.signal),
                ByteOffset::new(exit.final_offset),
            );
            status.observed &= exit.exit_unknown.is_none();
            status.io_failure = exit.io_failure.is_some();
            evidence.exit = Some(status);
        }
        Operations::new(&self.core, pending.context.at(now()?)).commit_unit(
            pending.permit,
            |snapshot| {
                let (_, version) = read_record(snapshot, record.id())?;
                Ok(Completion::new(serde_json::to_value(&receipt)?)
                    .evidence(evidence)
                    .write(CasWrite::new(
                        record_key(record.id()),
                        Version::new(version.get()),
                        Some(Payload::from(serde_json::to_vec(record)?)),
                    ))
                    .event(
                        if record.exit.is_some() {
                            LifecycleEvent::Exited(record.clone())
                        } else {
                            LifecycleEvent::Updated(record.clone())
                        },
                        vec![ResourceId::Terminal(record.id())],
                    ))
            },
        )?;
        Ok(())
    }
}
