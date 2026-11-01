//! The fixture's terminal owner for `terminal.process_info`: explicit targets only.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use ovis_terminal_agent::status::ProcessOwner;
use ovis_terminal_core::registry::{
    MachineRuntime, ResourceName, ResourceTarget, ResourceTargetKey, RuntimeIdentity, TargetIndex,
    TargetRequest, TargetResolutionError, TerminalId,
};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::records::ProcessRecord;
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};
use ovis_terminal_server::dispatch::lanes::process_info::{Ended, Launched, ProcessDirectory};
use serde_json::json;

use super::{PROC_TERMINAL, Programs, argv};
use crate::error::RunnerError;
use crate::fixture::serve::at;

/// Resolves `proc` by name or id and reads its program from the lifecycle service.
#[derive(Debug)]
pub(crate) struct ProcessDirectoryFixture {
    programs: Arc<Programs>,
    index: TargetIndex,
    scope: MachineRuntime,
}

impl ProcessDirectoryFixture {
    pub(crate) fn new(
        programs: Arc<Programs>,
        runtime: RuntimeIdentity,
    ) -> Result<Self, RunnerError> {
        let scope = MachineRuntime::new(runtime.machine_id(), runtime.runtime_id());
        let target = ResourceTarget::new(
            scope,
            TerminalId::try_from(PROC_TERMINAL).map_err(at("build a terminal id"))?,
            ResourceName::new("proc").map_err(at("build a terminal name"))?,
        );
        Ok(Self {
            programs,
            index: TargetIndex::from_targets(vec![target]),
            scope,
        })
    }

    fn record(
        &self,
        terminal: TerminalId,
    ) -> Result<ovis_terminal_core::terminals::Record, TargetResolutionError> {
        let guard = self
            .programs
            .terminals
            .lock()
            .map_err(|_| TargetResolutionError::NotFound)?;
        let record = guard
            .as_ref()
            .ok_or(TargetResolutionError::NotFound)
            .and_then(|terminals| {
                terminals
                    .get(&self.programs.session, terminal)
                    .map_err(|_| TargetResolutionError::NotFound)
            });
        drop(guard);
        record
    }
}

impl TerminalDirectory for ProcessDirectoryFixture {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        let request = match target {
            Target::TerminalId(id) => {
                TargetRequest::Id(ResourceTargetKey::new(self.scope, id.as_str().parse()?))
            }
            Target::Name(name) => TargetRequest::Name {
                machine: None,
                name: ResourceName::new(name.as_str())?,
            },
            _ => return Err(TargetResolutionError::NotFound),
        };
        let resolved = self.index.resolve(request)?;
        let record = self.record(resolved.terminal_id())?;
        Ok(ResolvedTerminal::new(
            resolved.machine_id(),
            resolved.terminal_id(),
            record.lifecycle.generation(),
        ))
    }
}

impl ProcessDirectory for ProcessDirectoryFixture {
    fn launched(&self, terminal: TerminalId) -> Result<Launched, TargetResolutionError> {
        let record = self.record(terminal)?;
        let last = record
            .lifecycle
            .last_process()
            .ok_or(TargetResolutionError::NotFound)?;
        let incarnation = last.generation().get();
        let started = self
            .programs
            .started
            .lock()
            .map_err(|_| TargetResolutionError::NotFound)?
            .get(&incarnation)
            .copied()
            .ok_or(TargetResolutionError::NotFound)?;
        let launch: ProcessRecord = serde_json::from_value(json!({
            "incarnation": incarnation.to_string(),
            "pid": u32::try_from(last.pid().get()).map_err(|_| TargetResolutionError::NotFound)?,
            "start_token": last.start_token().as_str(),
            "argv": argv(),
            "cwd": self.programs.cwd,
            "started_ms": started,
        }))
        .map_err(|_| TargetResolutionError::NotFound)?;
        let owned = record
            .lifecycle
            .process()
            .filter(|_| record.exit.is_none())
            .map(|process| process.handle().clone());
        Ok(Launched::new(launch, owned))
    }

    /// Retires the ended incarnation's report token by binding an unguessable one to it;
    /// a failure is reported when the fixture stops.
    fn ended(&self, ended: Ended) {
        let retired = ProcessToken::new(format!("retired-{}", uuid::Uuid::now_v7()));
        let owner = ProcessOwner::new(ended.terminal, ended.incarnation);
        if self
            .programs
            .service
            .tokens()
            .bind(&retired, owner)
            .is_err()
        {
            self.programs
                .revocation_failed
                .store(true, Ordering::SeqCst);
        }
    }
}
