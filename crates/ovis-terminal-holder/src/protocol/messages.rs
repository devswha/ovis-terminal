//! Frozen holder control message families.

mod control;
mod io;
mod lifecycle;
mod outcome;

pub use control::{Drain, Goodbye, GoodbyeReason, Message, MessageKind};
pub use io::{
    Exit, Forget, Hangup, Input, InputWritten, Output, Resize, Signal, SignalKind, Tail,
    TailRequest, TerminalTarget, WriteGrant,
};
pub use lifecycle::{
    HeldTerminal, Hello, PreviousCoreState, Spawn, SpawnFailed, SpawnFailureKind, Spawned, Welcome,
};
pub use outcome::{
    ExitUnknown, ExitUnknownReason, InputRejected, InputRejection, PtyIoFailure, PtyOperation,
    SpawnFailureReason,
};
