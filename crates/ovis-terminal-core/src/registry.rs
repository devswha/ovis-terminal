//! Identity resolution and lifecycle transition tables (plan todo 9).

mod authority;
mod ids;
mod lifecycle;
mod reservation;
mod resolution;

pub use authority::RegistryIdentities;
pub use ids::{
    DurableId, HostBootGeneration, HostBootId, IdentityError, LeaseGeneration, LeaseId,
    MachineGeneration, MachineId, PaneGeneration, PaneId, ProcessGeneration, ProcessId, ProcessPid,
    ProcessStartToken, ResourceName, RuntimeGeneration, RuntimeId, ServerBootGeneration,
    ServerBootId, SplitGeneration, SplitId, StreamGeneration, StreamId, TabGeneration, TabId,
    TerminalGeneration, TerminalId, WorkspaceGeneration, WorkspaceId,
};
pub use lifecycle::{
    HostIdentity, LifecycleEvent, LifecycleTransitionError, ProcessHandle, ProcessIdentity,
    RelaunchEvidence, RuntimeIdentity, SurvivalEvidence, TerminalLifecycle, TerminalState,
};
pub use reservation::{IdentityReservation, ReservationError, ReservedIdentity};
pub use resolution::{
    MachineRuntime, ResourceTarget, ResourceTargetKey, TargetIndex, TargetRequest,
    TargetResolutionError,
};
