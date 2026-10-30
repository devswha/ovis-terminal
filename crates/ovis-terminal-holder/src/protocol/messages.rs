//! Frozen holder control message families.

mod control;
mod io;
mod lifecycle;

pub use control::{Drain, Goodbye, GoodbyeReason, Message, MessageKind};
pub use io::{
    Exit, Forget, Input, InputWritten, Output, Resize, Signal, SignalKind, Tail, TailRequest,
    TerminalTarget, WriteGrant,
};
pub use lifecycle::{
    HeldTerminal, Hello, PreviousCoreState, Spawn, SpawnFailed, SpawnFailureKind, Spawned, Welcome,
};
