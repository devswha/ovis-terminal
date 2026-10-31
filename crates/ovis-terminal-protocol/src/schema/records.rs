//! Resource records shared by several method namespaces and events.

mod attach;
mod lease;
mod status;
mod stream;
mod terminal;

pub use attach::{AttachedTerminal, PaintedScreen, RetryBoundary, RetryOn};
pub use lease::{Checkpoint, LeaseRecord, LeaseRole, Modes, ScreenCursor, TerminalMode};
pub use status::{
    AgentBinding, AgentState, SessionRef, StatusEntry, StatusOrigin, StatusReport, StatusSnapshot,
};
pub use stream::{AttachControl, AttachData, ControlKind, SkippedOutput, StreamClose};
pub use terminal::{
    FinishReason, FinishedRecord, ParentRef, ProcessRecord, TerminalGeometry, TerminalOrigin,
    TerminalRecord, TerminalState,
};
