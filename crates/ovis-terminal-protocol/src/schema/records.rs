//! Resource records shared by several method namespaces and events.

mod lease;
mod status;
mod terminal;

pub use lease::{Checkpoint, LeaseRecord, LeaseRole, Modes, ScreenCursor, TerminalMode};
pub use status::{
    AgentBinding, AgentState, SessionRef, StatusEntry, StatusOrigin, StatusReport, StatusSnapshot,
};
pub use terminal::{
    FinishReason, FinishedRecord, ParentRef, ProcessRecord, TerminalGeometry, TerminalOrigin,
    TerminalRecord, TerminalState,
};
