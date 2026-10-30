//! Message discriminators and control-lifecycle messages.

use super::io::{Exit, Forget, Input, InputWritten, Output, Resize, Signal, Tail, TailRequest};
use super::lifecycle::{Hello, Spawn, SpawnFailed, Spawned, Welcome};

/// Stable message discriminator used by the holder frame codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MessageKind {
    /// Core begins adoption and presents the supported generation.
    Hello,
    /// Holder reports its identity and retained terminal records.
    Welcome,
    /// Core requests creation of a terminal process.
    Spawn,
    /// Holder confirms process creation.
    Spawned,
    /// Holder reports a typed spawn failure.
    SpawnFailed,
    /// Holder reports an ordered output range.
    Output,
    /// Core requests input with write-boundary authority evidence.
    Input,
    /// Holder acknowledges bytes actually written to the PTY.
    InputWritten,
    /// Core requests a terminal resize.
    Resize,
    /// Core requests a process signal.
    Signal,
    /// Holder reports process exit after all output through `final_offset`.
    Exit,
    /// Core requests or receives retained output.
    Tail,
    /// Core acknowledges removal of a retained terminal record.
    Forget,
    /// Core asks a holder to stop accepting new terminals and drain.
    Drain,
    /// Core reports why its previous ownership connection ended.
    Goodbye,
}

impl MessageKind {
    /// Returns the frozen version-one wire tag.
    #[must_use]
    pub const fn wire_tag(self) -> u8 {
        match self {
            Self::Hello => 1,
            Self::Welcome => 2,
            Self::Spawn => 3,
            Self::Spawned => 4,
            Self::SpawnFailed => 5,
            Self::Output => 6,
            Self::Input => 7,
            Self::InputWritten => 8,
            Self::Resize => 9,
            Self::Signal => 10,
            Self::Exit => 11,
            Self::Tail => 12,
            Self::Forget => 13,
            Self::Drain => 14,
            Self::Goodbye => 15,
        }
    }

    /// Parses a known version-one wire tag.
    ///
    /// # Errors
    ///
    /// Returns the unrecognized byte unchanged for unknown tags.
    pub const fn from_wire_tag(tag: u8) -> Result<Self, u8> {
        match tag {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Welcome),
            3 => Ok(Self::Spawn),
            4 => Ok(Self::Spawned),
            5 => Ok(Self::SpawnFailed),
            6 => Ok(Self::Output),
            7 => Ok(Self::Input),
            8 => Ok(Self::InputWritten),
            9 => Ok(Self::Resize),
            10 => Ok(Self::Signal),
            11 => Ok(Self::Exit),
            12 => Ok(Self::Tail),
            13 => Ok(Self::Forget),
            14 => Ok(Self::Drain),
            15 => Ok(Self::Goodbye),
            other => Err(other),
        }
    }
}

/// Holder drain lifecycle request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Drain {
    /// Stop new spawns while continuing to serve owned terminals.
    Begin,
    /// The core confirms no more adoption or drain traffic is required.
    Complete,
}

/// Core departure reason sent before a planned handoff or shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GoodbyeReason {
    /// The core has persisted recovery state for a successor.
    Handover,
    /// The runtime is stopping the holder service.
    Shutdown,
}

/// Core-to-holder departure notice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Goodbye {
    /// Explicit departure reason.
    pub reason: GoodbyeReason,
}

/// Closed set of control messages exchanged by core and holder.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Message {
    /// Adoption request.
    Hello(Hello),
    /// Adoption response.
    Welcome(Welcome),
    /// Spawn request.
    Spawn(Spawn),
    /// Spawn evidence.
    Spawned(Spawned),
    /// Spawn failure.
    SpawnFailed(SpawnFailed),
    /// Output range.
    Output(Output),
    /// Input request.
    Input(Input),
    /// Input write acknowledgement.
    InputWritten(InputWritten),
    /// Resize request.
    Resize(Resize),
    /// Signal request.
    Signal(Signal),
    /// Exit evidence.
    Exit(Exit),
    /// Tail request.
    TailRequest(TailRequest),
    /// Tail response.
    Tail(Tail),
    /// Forget acknowledgement.
    Forget(Forget),
    /// Drain lifecycle message.
    Drain(Drain),
    /// Core departure notice.
    Goodbye(Goodbye),
}

impl Message {
    /// Returns the frozen frame discriminator for this message.
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        match self {
            Self::Hello(_) => MessageKind::Hello,
            Self::Welcome(_) => MessageKind::Welcome,
            Self::Spawn(_) => MessageKind::Spawn,
            Self::Spawned(_) => MessageKind::Spawned,
            Self::SpawnFailed(_) => MessageKind::SpawnFailed,
            Self::Output(_) => MessageKind::Output,
            Self::Input(_) => MessageKind::Input,
            Self::InputWritten(_) => MessageKind::InputWritten,
            Self::Resize(_) => MessageKind::Resize,
            Self::Signal(_) => MessageKind::Signal,
            Self::Exit(_) => MessageKind::Exit,
            Self::TailRequest(_) | Self::Tail(_) => MessageKind::Tail,
            Self::Forget(_) => MessageKind::Forget,
            Self::Drain(_) => MessageKind::Drain,
            Self::Goodbye(_) => MessageKind::Goodbye,
        }
    }
}
