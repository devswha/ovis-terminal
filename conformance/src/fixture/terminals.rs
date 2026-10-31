//! `terminal.keep` over a real target index: two terminals share a name, one has its own.
//! The effect file is named after the terminal the explicit target resolved to.

use std::path::{Path, PathBuf};

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{
    MachineRuntime, ResourceName, ResourceTarget, ResourceTargetKey, RuntimeIdentity, TargetIndex,
    TargetRequest, TargetResolutionError, TerminalGeneration, TerminalId,
};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::terminal::TerminalKeepParams;
use ovis_terminal_server::dispatch::{Access, Completed, DispatchError, Execution, Method};
use serde_json::json;

use super::handler::Event;
use super::serve::at;
use crate::error::RunnerError;

/// Terminal ids 1 and 2 are both named `shared`; id 3 is `solo`.
const TERMINALS: [(u64, &str); 3] = [(1, "shared"), (2, "shared"), (3, "solo")];

#[derive(Debug)]
pub(crate) struct TerminalKeep {
    index: TargetIndex,
    scope: MachineRuntime,
    effects: PathBuf,
    generation: TerminalGeneration,
}

impl TerminalKeep {
    pub(crate) fn new(effects: &Path, runtime: RuntimeIdentity) -> Result<Self, RunnerError> {
        let scope = MachineRuntime::new(runtime.machine_id(), runtime.runtime_id());
        let mut targets = Vec::with_capacity(TERMINALS.len());
        for (id, name) in TERMINALS {
            targets.push(ResourceTarget::new(
                scope,
                TerminalId::try_from(id).map_err(at("build a terminal id"))?,
                ResourceName::new(name).map_err(at("build a terminal name"))?,
            ));
        }
        Ok(Self {
            index: TargetIndex::from_targets(targets),
            scope,
            effects: effects.to_path_buf(),
            generation: TerminalGeneration::new(1).map_err(at("build a generation"))?,
        })
    }

    fn request(&self, target: &Target) -> Result<TargetRequest, TargetResolutionError> {
        match target {
            Target::TerminalId(id) => Ok(TargetRequest::Id(ResourceTargetKey::new(
                self.scope,
                id.as_str().parse()?,
            ))),
            Target::Name(name) => Ok(TargetRequest::Name {
                machine: None,
                name: ResourceName::new(name.as_str())?,
            }),
            Target::Address(_) => Err(TargetResolutionError::NotFound),
            _ => Err(TargetResolutionError::NeedsTarget),
        }
    }
}

impl Method<Event, OperationProducer> for TerminalKeep {
    const NAME: &'static str = "terminal.keep";
    type Params = TerminalKeepParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let resolved = self.index.resolve(self.request(&params.target)?)?;
        Ok(Access::new(
            Action::Input,
            ResolvedTarget::new(
                resolved.machine_id(),
                TargetResource::Terminal {
                    placement: None,
                    terminal: resolved.terminal_id(),
                    incarnation: self.generation,
                },
            ),
        ))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<Event, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let TargetResource::Terminal { terminal, .. } = execution.target().resource() else {
            return Err(DispatchError::Registration);
        };
        let terminal = *terminal;
        let path = self.effects.join(format!("{terminal}.json"));
        execution
            .commit(move || {
                let record = json!({ "terminal_id": terminal, "keep": params.keep });
                std::fs::write(path, serde_json::to_vec(&record)?)?;
                Ok(record)
            })
            .await
    }
}
