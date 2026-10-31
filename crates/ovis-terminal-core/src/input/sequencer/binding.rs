//! The terminal incarnation a sequencer serves and the identities a request must match.

use std::sync::Arc;

use ovis_terminal_authz::{Authority, ResolvedTarget, TargetResource};
use ovis_terminal_holder::protocol::{
    ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation, TerminalTarget,
};

use super::SequencerError;
use crate::registry::{ProcessGeneration, RegistryIdentities, RuntimeId};

type Registry = RegistryIdentities;

/// The authority deciding writes and the one terminal incarnation they go to.
#[derive(Debug)]
pub struct Binding {
    authority: Arc<Authority<Registry>>,
    runtime: RuntimeId,
    target: ResolvedTarget<Registry>,
    holder: (HolderTerminalId, TerminalIncarnation),
}

impl Binding {
    /// Binds input of `target` in `runtime` to `authority`.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` when `target` is not a terminal.
    pub fn new(
        authority: Arc<Authority<Registry>>,
        runtime: RuntimeId,
        target: ResolvedTarget<Registry>,
    ) -> Result<Self, SequencerError> {
        let TargetResource::Terminal {
            terminal,
            incarnation,
            ..
        } = target.resource()
        else {
            return Err(SequencerError::NotTerminal);
        };
        let holder = (
            HolderTerminalId::new(terminal.to_string()),
            TerminalIncarnation::new(incarnation.get()),
        );
        Ok(Self {
            authority,
            runtime,
            target,
            holder,
        })
    }

    pub(super) fn authority(&self) -> &Authority<Registry> {
        &self.authority
    }

    pub(super) const fn target(&self) -> &ResolvedTarget<Registry> {
        &self.target
    }

    pub(super) fn holder_target(&self, process: ProcessGeneration) -> TerminalTarget {
        TerminalTarget {
            terminal_id: self.holder.0.clone(),
            terminal_incarnation: self.holder.1,
            process_incarnation: Some(ProcessIncarnation::new(process.get())),
        }
    }

    /// Fails unless `expected` still names this terminal and its running process.
    pub(super) fn check(
        &self,
        expected: &Expected,
        current: ProcessGeneration,
    ) -> Result<(), SequencerError> {
        let stale = |field| Err(SequencerError::StaleTarget { field });
        if expected.runtime != self.runtime {
            return stale("runtime");
        }
        if expected.target.machine() != self.target.machine() {
            return stale("machine");
        }
        match (expected.target.resource(), self.target.resource()) {
            (
                TargetResource::Terminal {
                    placement,
                    terminal,
                    incarnation,
                },
                TargetResource::Terminal {
                    placement: bound_placement,
                    terminal: bound_terminal,
                    incarnation: bound_incarnation,
                },
            ) => {
                if terminal != bound_terminal {
                    return stale("terminal");
                }
                if incarnation != bound_incarnation {
                    return stale("terminal_incarnation");
                }
                if placement != bound_placement {
                    return stale("pane");
                }
            }
            _ => return stale("terminal"),
        }
        match expected.process {
            Some(process) if process != current => Err(SequencerError::ProcessReplaced {
                expected: process,
                current,
            }),
            _ => Ok(()),
        }
    }
}

/// Identities a request was resolved against; each must still hold at admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    runtime: RuntimeId,
    target: ResolvedTarget<Registry>,
    process: Option<ProcessGeneration>,
}

impl Expected {
    /// The target the request resolved to in `runtime`, with no process expectation.
    pub const fn new(runtime: RuntimeId, target: ResolvedTarget<Registry>) -> Self {
        Self {
            runtime,
            target,
            process: None,
        }
    }

    /// Also requires the running process incarnation to be `process`.
    #[must_use]
    pub const fn with_process(mut self, process: ProcessGeneration) -> Self {
        self.process = Some(process);
        self
    }
}
