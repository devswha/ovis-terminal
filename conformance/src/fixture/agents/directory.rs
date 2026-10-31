//! The fixture's terminal registry and the terminals it starts with.

use std::sync::Mutex;

use ovis_terminal_core::registry::{
    MachineRuntime, ResourceName, ResourceTarget, ResourceTargetKey, RuntimeIdentity, TargetIndex,
    TargetRequest, TargetResolutionError, TerminalGeneration, TerminalId,
};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};

use super::TERMINALS;
use crate::error::RunnerError;
use crate::fixture::serve::at;

/// The fixture's terminal registry; it never selects a focused terminal. Launched
/// terminals join it under their names.
#[derive(Debug)]
pub(crate) struct Directory {
    targets: Mutex<Vec<ResourceTarget>>,
    scope: MachineRuntime,
    generation: TerminalGeneration,
}

impl Directory {
    /// Adds a launched terminal named `name`.
    pub(crate) fn add(&self, terminal: TerminalId, name: ResourceName) {
        let target = ResourceTarget::new(self.scope, terminal, name);
        self.targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(target);
    }

    /// Whether a terminal already carries `name`.
    pub(crate) fn named(&self, name: &str) -> bool {
        ResourceName::new(name).is_ok_and(|name| {
            TargetIndex::from_targets(self.snapshot())
                .resolve(TargetRequest::Name {
                    machine: None,
                    name,
                })
                .is_ok()
        })
    }

    fn snapshot(&self) -> Vec<ResourceTarget> {
        self.targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl TerminalDirectory for Directory {
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
        let resolved = TargetIndex::from_targets(self.snapshot()).resolve(request)?;
        Ok(ResolvedTerminal::new(
            resolved.machine_id(),
            resolved.terminal_id(),
            self.generation,
        ))
    }
}

pub(super) fn directory(runtime: RuntimeIdentity) -> Result<Directory, RunnerError> {
    let scope = MachineRuntime::new(runtime.machine_id(), runtime.runtime_id());
    let mut targets = Vec::with_capacity(TERMINALS.len());
    for (id, name, _) in TERMINALS {
        targets.push(ResourceTarget::new(
            scope,
            TerminalId::try_from(id).map_err(at("build a terminal id"))?,
            ResourceName::new(name).map_err(at("build a terminal name"))?,
        ));
    }
    Ok(Directory {
        targets: Mutex::new(targets),
        scope,
        generation: TerminalGeneration::new(1).map_err(at("build a generation"))?,
    })
}
