//! Authorized metadata changes do not write to or signal a terminal.

use ovis_terminal_authz::{AccessRequest, Action};
use ovis_terminal_holder::protocol::Forget;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::{CasWrite, Payload, Version};
use serde::Serialize;

use super::{Call, Terminals, now};
use crate::events::ResourceId;
use crate::ops::{Completion, OperationError, Request};
use crate::registry::{ResourceName, TerminalId};
use crate::terminals::snapshot::{index_key, read_index, read_record, record_key};
use crate::terminals::{LabelRequest, LifecycleEvent, Record, RemovalReceipt, TerminalError};

#[derive(Debug, Serialize)]
enum Change {
    Keep(bool),
    Rename(Option<ResourceName>),
    Labels(LabelRequest),
}

impl Terminals {
    /// Changes retention policy without restarting or stopping a program.
    ///
    /// # Errors
    ///
    /// Returns authority, deduplication or persistence errors.
    pub fn keep(
        &mut self,
        call: Call<'_>,
        (id, keep): (TerminalId, bool),
    ) -> Result<Record, TerminalError> {
        self.metadata(call, (id, Change::Keep(keep)))
    }

    /// Renames an explicit terminal without redirecting any process binding.
    ///
    /// # Errors
    ///
    /// Returns `duplicate_name`, authority or persistence errors.
    pub fn rename(
        &mut self,
        call: Call<'_>,
        (id, name): (TerminalId, Option<ResourceName>),
    ) -> Result<Record, TerminalError> {
        self.metadata(call, (id, Change::Rename(name)))
    }

    /// Sets and removes label keys without program side effects.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params`, authority or persistence errors.
    pub fn label(&mut self, call: Call<'_>, labels: LabelRequest) -> Result<Record, TerminalError> {
        if labels
            .set
            .keys()
            .chain(labels.remove.iter())
            .any(|key| key.is_empty() || key.contains('='))
        {
            return Err(TerminalError::InvalidParams);
        }
        self.metadata(call, (labels.terminal, Change::Labels(labels)))
    }

    fn metadata(
        &self,
        call: Call<'_>,
        (id, change): (TerminalId, Change),
    ) -> Result<Record, TerminalError> {
        self.ensure_running()?;
        let mut record = self.record(id)?;
        let decision = self.authorize(call.session, (&record, Action::Administer))?;
        let method = match &change {
            Change::Keep(_) => "terminal.keep",
            Change::Rename(_) => "terminal.rename",
            Change::Labels(_) => "terminal.label",
        };
        let request = Request::new(
            call.op_id,
            method.into(),
            serde_json::json!({"terminal":id,"change":change}),
        )?;
        let operations = self.operations(call.session)?;
        let accepted = operations.accept(&request)?;
        if accepted.state == OpenEnum::Known(OpState::Committed) {
            return Ok(serde_json::from_value(
                accepted.result.ok_or(TerminalError::NotFound)?,
            )?);
        }
        let permit = operations.start(request.op_id())?;
        match change {
            Change::Keep(keep) => record.launch.keep = keep,
            Change::Rename(name) => {
                if let Some(name) = &name
                    && self
                        .core
                        .read_snapshot()?
                        .sections
                        .terminals
                        .iter()
                        .any(|other| {
                            other.id() != id
                                && other.launch.name.as_ref() == Some(name)
                                && other.exit.is_none()
                        })
                {
                    return Err(TerminalError::DuplicateName);
                }
                record.launch.name = name;
            }
            Change::Labels(labels) => {
                for key in labels.remove {
                    record.launch.labels.remove(&key);
                }
                record.launch.labels.extend(labels.set);
            }
        }
        operations.commit_unit(permit, |snapshot| {
            self.host.authority.recheck(
                &decision,
                &AccessRequest::new(Action::Administer, self.target(&record), now()?),
            )?;
            let (_, version) = read_record(snapshot, id)?;
            super::stamp_operation(&mut record, snapshot)?;
            Ok(Completion::new(serde_json::to_value(&record)?)
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
        self.record(id)
    }

    /// Tombstones finished metadata; this method never delivers a signal.
    ///
    /// # Errors
    ///
    /// Returns `still_live` without a holder effect, or authority/store errors.
    pub fn rm(&mut self, call: Call<'_>, id: TerminalId) -> Result<RemovalReceipt, TerminalError> {
        self.ensure_running()?;
        let operations = self.operations(call.session)?;
        let request = Request::new(
            call.op_id,
            "terminal.rm".into(),
            serde_json::json!({"terminal":id}),
        )?;
        match operations.get(request.op_id()) {
            Ok(prior) if prior.state == OpenEnum::Known(OpState::Committed) => {
                let retained = operations.accept(&request)?;
                let receipt: RemovalReceipt =
                    serde_json::from_value(retained.result.ok_or(TerminalError::NotFound)?)?;
                self.host.authority.authorize(
                    call.session,
                    &AccessRequest::new(
                        Action::Administer,
                        ovis_terminal_authz::ResolvedTarget::new(
                            self.host.identity.machine_id(),
                            ovis_terminal_authz::TargetResource::Terminal {
                                placement: None,
                                terminal: id,
                                incarnation: receipt.terminal_incarnation,
                            },
                        ),
                        now()?,
                    ),
                )?;
                return Ok(receipt);
            }
            Ok(_) | Err(OperationError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        let record = self.record(id)?;
        let decision = self.authorize(call.session, (&record, Action::Administer))?;
        if !Self::finished(&record) && record.spawn != crate::terminals::LaunchState::Failed {
            return Err(TerminalError::StillLive);
        }
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        let receipt = RemovalReceipt {
            terminal: id,
            terminal_incarnation: record.lifecycle.generation(),
            affected: record
                .lifecycle
                .last_process()
                .cloned()
                .into_iter()
                .collect(),
        };
        operations.commit_unit(permit, |snapshot| {
            self.host.authority.recheck(
                &decision,
                &AccessRequest::new(Action::Administer, self.target(&record), now()?),
            )?;
            let (_, version) = read_record(snapshot, id)?;
            let (mut ids, index_version) = read_index(snapshot, self.host.identity.runtime_id())?;
            ids.retain(|terminal| *terminal != id);
            Ok(Completion::new(serde_json::to_value(&receipt)?)
                .write(CasWrite::new(
                    record_key(id),
                    Version::new(version.get()),
                    None,
                ))
                .write(CasWrite::new(
                    index_key(self.host.identity.runtime_id()),
                    Version::new(index_version.get()),
                    Some(Payload::from(serde_json::to_vec(&ids)?)),
                ))
                .event(
                    LifecycleEvent::Removed {
                        terminal: id,
                        process: record.lifecycle.last_process().cloned(),
                    },
                    vec![ResourceId::Terminal(id)],
                ))
        })?;
        if let Some(exit) = record.exit {
            self.host.holder.forget(&Forget {
                target: exit.target,
                acknowledged_final_offset: Some(exit.final_offset),
            })?;
        }
        Ok(receipt)
    }
}
