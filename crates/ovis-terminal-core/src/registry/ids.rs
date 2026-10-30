//! Public re-exports for semantic identifiers and their boundary errors.

mod error;
mod generations;
mod machine;
mod names;
mod resources;

#[cfg(test)]
mod tests;

pub use error::IdentityError;
pub use generations::{
    HostBootGeneration, LeaseGeneration, MachineGeneration, PaneGeneration, ProcessGeneration,
    RuntimeGeneration, ServerBootGeneration, SplitGeneration, StreamGeneration, TabGeneration,
    TerminalGeneration, WorkspaceGeneration,
};
pub use machine::MachineId;
pub use names::{ProcessPid, ProcessStartToken, ResourceName};
pub use resources::{
    DurableId, HostBootId, LeaseId, PaneId, ProcessId, RuntimeId, ServerBootId, SplitId, StreamId,
    TabId, TerminalId, WorkspaceId,
};
