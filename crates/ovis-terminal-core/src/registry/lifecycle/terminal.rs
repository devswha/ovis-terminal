//! State and incarnation history for one terminal.

mod persisted;

use super::{
    LifecycleEvent, LifecycleTransitionError, ProcessIdentity, RelaunchEvidence, RuntimeIdentity,
    SurvivalEvidence, TerminalState,
};
use crate::registry::{ProcessGeneration, TerminalGeneration, TerminalId};

/// The state and incarnation history for one opaque terminal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "persisted::PersistedLifecycle")]
pub struct TerminalLifecycle {
    identity: RuntimeIdentity,
    id: TerminalId,
    generation: TerminalGeneration,
    state: TerminalState,
    current_process: Option<ProcessIdentity>,
    last_process_generation: Option<ProcessGeneration>,
    previous_process: Option<ProcessIdentity>,
}

impl TerminalLifecycle {
    /// Creates terminal metadata before any process is claimed to be running.
    pub const fn new(
        identity: RuntimeIdentity,
        id: TerminalId,
        generation: TerminalGeneration,
    ) -> Self {
        Self {
            identity,
            id,
            generation,
            state: TerminalState::RestoredMetadata,
            current_process: None,
            last_process_generation: None,
            previous_process: None,
        }
    }

    /// Stable terminal resource identity.
    pub const fn terminal_id(&self) -> TerminalId {
        self.id
    }

    /// Runtime lifetime containing this terminal.
    pub const fn runtime_id(&self) -> crate::registry::RuntimeId {
        self.identity.runtime_id()
    }

    /// Machine, host, runtime and control boot backing this lifecycle.
    pub const fn identity(&self) -> RuntimeIdentity {
        self.identity
    }

    /// Current terminal incarnation.
    pub const fn generation(&self) -> TerminalGeneration {
        self.generation
    }

    /// Current terminal lifecycle state.
    pub const fn state(&self) -> TerminalState {
        self.state
    }

    /// Current process identity, if this terminal has a proven live process.
    pub const fn process(&self) -> Option<&ProcessIdentity> {
        self.current_process.as_ref()
    }

    /// Last owned process, retained after loss or an evidenced exit.
    pub fn last_process(&self) -> Option<&ProcessIdentity> {
        self.current_process
            .as_ref()
            .or(self.previous_process.as_ref())
    }

    /// The previous process retained across a confirmed relaunch.
    pub const fn previous_process(&self) -> Option<&ProcessIdentity> {
        self.previous_process.as_ref()
    }

    /// Applies one lifecycle event without claiming unproven survival or relaunch.
    ///
    /// # Errors
    ///
    /// Returns a typed transition error when evidence is missing or generations do not advance.
    pub fn transition(&self, event: LifecycleEvent) -> Result<Self, LifecycleTransitionError> {
        let mut next = self.clone();
        match event {
            LifecycleEvent::ProcessEnded => {
                if next.state != TerminalState::Running {
                    return Err(LifecycleTransitionError::InvalidTransition);
                }
                next.previous_process = next.current_process.take();
                next.state = TerminalState::Finished;
            }
            LifecycleEvent::InitialSpawn { evidence, process } => {
                match evidence {
                    RelaunchEvidence::SpawnConfirmed => {}
                }
                if next.state != TerminalState::RestoredMetadata
                    || next.last_process_generation.is_some()
                    || process.generation() != ProcessGeneration::new(1)?
                {
                    return Err(LifecycleTransitionError::InvalidTransition);
                }
                next.last_process_generation = Some(process.generation());
                next.current_process = Some(process);
                next.state = TerminalState::Running;
            }
            LifecycleEvent::CoreRestart {
                server_boot_id,
                survival,
            }
            | LifecycleEvent::Upgrade {
                server_boot_id,
                survival,
            } => {
                if server_boot_id == next.identity.server_boot_id() {
                    return Err(LifecycleTransitionError::InvalidTransition);
                }
                let evidence_matches = match (&next.current_process, survival) {
                    (Some(current), SurvivalEvidence::HolderConfirmed(proven)) => {
                        current == &proven
                    }
                    (None, SurvivalEvidence::NoLiveProcess) => true,
                    (Some(_), SurvivalEvidence::NoLiveProcess)
                    | (None, SurvivalEvidence::HolderConfirmed(_)) => false,
                };
                if !evidence_matches {
                    return Err(LifecycleTransitionError::SurvivalMismatch);
                }
                next.identity.server_boot = server_boot_id;
            }
            LifecycleEvent::HolderLost => {
                if next.state != TerminalState::Running {
                    return Err(LifecycleTransitionError::InvalidTransition);
                }
                next.previous_process = next.current_process.take();
                next.state = TerminalState::Lost;
            }
            LifecycleEvent::HostReboot { identity } => {
                if identity.host_boot_id() == next.identity.host_boot_id()
                    || identity.runtime_id() == next.identity.runtime_id()
                    || identity.server_boot_id() == next.identity.server_boot_id()
                    || identity.machine_id() != next.identity.machine_id()
                {
                    return Err(LifecycleTransitionError::RebootIdentityUnchanged);
                }
                next.previous_process = next.current_process.take().or(next.previous_process);
                next.identity = identity;
                next.state = TerminalState::RestoredMetadata;
            }
            LifecycleEvent::Relaunch { evidence, process } => {
                match evidence {
                    RelaunchEvidence::SpawnConfirmed => {}
                }
                if next.current_process.is_some() {
                    return Err(LifecycleTransitionError::AlreadyRunning);
                }
                let previous_generation = next
                    .last_process_generation
                    .ok_or(LifecycleTransitionError::InvalidTransition)?;
                let expected_process_generation = previous_generation
                    .checked_next()
                    .map_err(|_| LifecycleTransitionError::ProcessGenerationOverflow)?;
                if process.generation() != expected_process_generation {
                    return Err(LifecycleTransitionError::ProcessGenerationMismatch);
                }
                if next.previous_process.as_ref().is_some_and(|previous| {
                    previous.pid() == process.pid()
                        && previous.start_token() == process.start_token()
                }) {
                    return Err(LifecycleTransitionError::ProcessGenerationMismatch);
                }
                next.generation = next
                    .generation
                    .checked_next()
                    .map_err(|_| LifecycleTransitionError::TerminalGenerationOverflow)?;
                next.last_process_generation = Some(process.generation());
                next.current_process = Some(process);
                next.state = TerminalState::Running;
            }
        }
        Ok(next)
    }

    /// Resolves an exact process identity without falling through to a replacement.
    ///
    /// # Errors
    ///
    /// Returns `stale_target` when the supplied PID/start-token pair is not current.
    pub fn resolve_process(
        &self,
        expected: &ProcessIdentity,
    ) -> Result<&ProcessIdentity, LifecycleTransitionError> {
        match self.current_process.as_ref() {
            Some(current) if current == expected => Ok(current),
            actual => Err(LifecycleTransitionError::StaleTarget {
                expected: expected.clone(),
                actual: actual.cloned(),
            }),
        }
    }
}
