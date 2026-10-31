//! Runtime identity and process lifecycle types.

mod terminal;

use serde::{Deserialize, Serialize};
pub use terminal::TerminalLifecycle;

use crate::registry::{
    HostBootId, IdentityError, MachineId, ProcessGeneration, ProcessId, ProcessPid,
    ProcessStartToken, RuntimeId, ServerBootId,
};

/// Stable machine and operating-system boot identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIdentity {
    machine: MachineId,
    host_boot: HostBootId,
}

impl HostIdentity {
    /// Creates the identity of one installed machine on one OS boot.
    pub const fn new(machine: MachineId, host_boot: HostBootId) -> Self {
        Self { machine, host_boot }
    }

    /// Stable machine identity.
    pub const fn machine_id(self) -> MachineId {
        self.machine
    }

    /// Operating-system boot identity.
    pub const fn host_boot_id(self) -> HostBootId {
        self.host_boot
    }
}

/// Identity scope for a running control server and its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    host: HostIdentity,
    runtime: RuntimeId,
    server_boot: ServerBootId,
}

impl RuntimeIdentity {
    /// Creates one complete machine/runtime/host-boot/server-boot identity.
    pub const fn new(host: HostIdentity, runtime: RuntimeId, server_boot: ServerBootId) -> Self {
        Self {
            host,
            runtime,
            server_boot,
        }
    }

    /// Stable machine identity.
    pub const fn machine_id(self) -> MachineId {
        self.host.machine_id()
    }

    /// Operating-system boot identity.
    pub const fn host_boot_id(self) -> HostBootId {
        self.host.host_boot_id()
    }

    /// Runtime lifetime identity, stable across control-core restart.
    pub const fn runtime_id(self) -> RuntimeId {
        self.runtime
    }

    /// Control-server process identity, changed on every start.
    pub const fn server_boot_id(self) -> ServerBootId {
        self.server_boot
    }
}

/// Evidence that a replacement process was actually spawned and identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RelaunchEvidence {
    /// A holder reported a completed spawn with a PID and start token.
    SpawnConfirmed,
}

/// The lifecycle cause used to advance a terminal's identity state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LifecycleEvent {
    /// The owning holder reaped the process and supplied its final exit evidence.
    ProcessEnded,
    /// The first process was confirmed after a terminal resource was allocated.
    InitialSpawn {
        /// Proof that the holder completed the first process spawn.
        evidence: RelaunchEvidence,
        /// Newly observed process identity including PID and start token.
        process: ProcessIdentity,
    },
    /// The core restarted while the host, runtime and holder remain alive.
    CoreRestart {
        /// Newly allocated control-server boot identity.
        server_boot_id: ServerBootId,
        /// Holder evidence for this terminal's process at adoption.
        survival: SurvivalEvidence,
    },
    /// An upgrade replaced the core while holders continue draining processes.
    Upgrade {
        /// Newly allocated control-server boot identity.
        server_boot_id: ServerBootId,
        /// Holder evidence for this terminal's process at adoption.
        survival: SurvivalEvidence,
    },
    /// The holder owning this terminal was confirmed lost.
    HolderLost,
    /// Independent OS evidence proves the host rebooted.
    HostReboot {
        /// Identity recovered for the new host/runtime/server boot.
        identity: RuntimeIdentity,
    },
    /// A holder confirmed the replacement process spawn.
    Relaunch {
        /// Proof required before an incarnation can advance.
        evidence: RelaunchEvidence,
        /// Newly observed process identity including PID and start token.
        process: ProcessIdentity,
    },
}

/// Process evidence captured while a replacement core adopts a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurvivalEvidence {
    /// The holder confirms that this exact process identity is still live.
    HolderConfirmed(ProcessIdentity),
    /// The terminal had no active process when the replacement core adopted it.
    NoLiveProcess,
}

/// Lifecycle state of a terminal resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalState {
    /// A process incarnation is currently owned by a live holder.
    Running,
    /// The holder or process was lost and no replacement was proven.
    Lost,
    /// The host rebooted; persisted metadata exists but no process is claimed alive.
    RestoredMetadata,
    /// The holder reaped the last process; its identity remains historical evidence.
    Finished,
}

/// Proof and transition errors at the lifecycle boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LifecycleTransitionError {
    /// A host reboot transition did not provide a new runtime and host boot.
    #[error("host reboot evidence did not identify a new runtime and host boot")]
    RebootIdentityUnchanged,
    /// A replacement process did not advance its generation.
    #[error("the replacement process generation did not advance exactly once")]
    ProcessGenerationMismatch,
    /// A terminal generation could not be advanced safely.
    #[error("the terminal generation cannot be advanced")]
    TerminalGenerationOverflow,
    /// A process generation could not be advanced safely.
    #[error("the process generation cannot be advanced")]
    ProcessGenerationOverflow,
    /// Holder adoption evidence does not prove the current process identity.
    #[error("holder adoption evidence does not match the stored process identity")]
    SurvivalMismatch,
    /// A relaunch was requested while a process is already live.
    #[error("a process incarnation is already live")]
    AlreadyRunning,
    /// The supplied process token identifies no current process incarnation.
    #[error("the requested process incarnation is no longer current")]
    StaleTarget {
        /// Expected PID and start token from the caller.
        expected: ProcessIdentity,
        /// Current PID and start token, if a replacement is running.
        actual: Option<ProcessIdentity>,
    },
    /// A lifecycle transition is invalid for the current state.
    #[error("the transition is not valid in the current terminal state")]
    InvalidTransition,
    /// An identity component is malformed.
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

impl LifecycleTransitionError {
    /// Stable machine-readable error kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::RebootIdentityUnchanged | Self::InvalidTransition => "invalid_transition",
            Self::ProcessGenerationMismatch | Self::SurvivalMismatch | Self::StaleTarget { .. } => {
                "stale_target"
            }
            Self::TerminalGenerationOverflow | Self::ProcessGenerationOverflow => {
                "counter_overflow"
            }
            Self::AlreadyRunning => "already_running",
            Self::Identity(error) => error.kind(),
        }
    }

    /// Recovery guidance for the error kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::RebootIdentityUnchanged => {
                "supply independently observed host-boot and runtime identities"
            }
            Self::ProcessGenerationMismatch => {
                "read the latest process identity and retry only after a confirmed spawn"
            }
            Self::TerminalGenerationOverflow | Self::ProcessGenerationOverflow => {
                "allocate a new terminal identity instead of wrapping an incarnation"
            }
            Self::SurvivalMismatch => {
                "refresh holder evidence; the stored process was not reported as surviving"
            }
            Self::AlreadyRunning => "wait for the current process to finish before relaunch",
            Self::StaleTarget { .. } => {
                "refresh the process PID and start token; do not redirect the request"
            }
            Self::InvalidTransition => "read the current lifecycle state before retrying",
            Self::Identity(error) => error.hint(),
        }
    }
}

/// Explicit process identity with an OS PID and PID-reuse discriminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    id: ProcessId,
    generation: ProcessGeneration,
    process: ProcessHandle,
}

/// Operating-system process evidence, including the PID-reuse discriminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHandle {
    pid: ProcessPid,
    start_token: ProcessStartToken,
}

impl ProcessHandle {
    /// Creates OS process evidence from the PID and its start token.
    pub const fn new(pid: ProcessPid, start_token: ProcessStartToken) -> Self {
        Self { pid, start_token }
    }

    /// Operating-system PID.
    pub const fn pid(&self) -> ProcessPid {
        self.pid
    }

    /// Operating-system start token.
    pub const fn start_token(&self) -> &ProcessStartToken {
        &self.start_token
    }
}

impl ProcessIdentity {
    /// Creates an identity that distinguishes operating-system PID reuse.
    pub const fn new(id: ProcessId, generation: ProcessGeneration, process: ProcessHandle) -> Self {
        Self {
            id,
            generation,
            process,
        }
    }

    /// Opaque runtime process identity.
    pub const fn process_id(&self) -> ProcessId {
        self.id
    }

    /// Native PID and start token, independent of the registry's process resource ID.
    pub const fn handle(&self) -> &ProcessHandle {
        &self.process
    }

    /// Process incarnation generation.
    pub const fn generation(&self) -> ProcessGeneration {
        self.generation
    }

    /// Operating-system PID.
    pub const fn pid(&self) -> ProcessPid {
        self.process.pid()
    }

    /// Operating-system start token.
    pub const fn start_token(&self) -> &ProcessStartToken {
        self.process.start_token()
    }
}
