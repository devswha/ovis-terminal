//! Holder adoption and process lifecycle records.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::io::Exit;
use super::outcome::SpawnFailureReason;
use crate::protocol::{
    CoreBuildId, HolderBuildId, HolderInstanceId, ProcessIncarnation, ProcessStartToken,
    TerminalId, TerminalIncarnation,
};

/// Previous-core condition included in a hello.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PreviousCoreState {
    /// The core intentionally saved state before handoff.
    Handover,
    /// The core disconnected without a planned handoff.
    Crash,
}

/// First message sent by a core during holder adoption.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Hello {
    /// Independently versioned holder protocol generation.
    #[serde(with = "crate::protocol::decimal_u16")]
    pub generation: u16,
    /// Build the adopting core can service.
    pub core_build: CoreBuildId,
    /// Whether the previous core disconnected intentionally.
    pub previous: PreviousCoreState,
}

/// A terminal and process record currently owned by the holder.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct HeldTerminal {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Incarnation of the terminal record.
    pub terminal_incarnation: TerminalIncarnation,
    /// Process incarnation, present only when a process was spawned.
    pub process_incarnation: Option<ProcessIncarnation>,
    /// PID of the currently held process.
    #[serde(default, with = "crate::protocol::optional_decimal_u32")]
    pub pid: Option<u32>,
    /// OS token preventing a reused PID from being mistaken for the process.
    pub start_token: Option<ProcessStartToken>,
    /// Last contiguous raw output byte offset.
    #[serde(with = "crate::protocol::decimal")]
    pub output_offset: u64,
    /// Highest input sequence known to have reached its write boundary.
    #[serde(with = "crate::protocol::decimal")]
    pub input_sequence: u64,
    /// Final exit record, retained until the core acknowledges it.
    pub exit: Option<Exit>,
}

/// Holder response to a supported hello, including its adoption snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Welcome {
    /// Negotiated holder protocol generation.
    #[serde(with = "crate::protocol::decimal_u16")]
    pub generation: u16,
    /// Executable build identity.
    pub build: HolderBuildId,
    /// This holder process lifetime.
    pub instance: HolderInstanceId,
    /// Records that the core must reconcile before taking ownership.
    pub terminals: Vec<HeldTerminal>,
}

/// Request to spawn a process in a newly allocated terminal identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Spawn {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Initial terminal incarnation.
    pub terminal_incarnation: TerminalIncarnation,
    /// Executable (first element, resolved with the explicit `PATH`) and arguments.
    pub argv: Vec<String>,
    /// Working directory the program starts in; it must exist.
    pub cwd: PathBuf,
    /// The program's complete environment; nothing is inherited from the holder.
    pub env: BTreeMap<String, String>,
    /// Initial terminal geometry as columns, rows.
    pub geometry: (u16, u16),
}

/// Successful spawn evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Spawned {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Terminal incarnation that received the process.
    pub terminal_incarnation: TerminalIncarnation,
    /// New process incarnation.
    pub process_incarnation: ProcessIncarnation,
    /// OS process identifier.
    #[serde(with = "crate::protocol::decimal_u32")]
    pub pid: u32,
    /// OS token distinguishing this process from PID reuse.
    pub start_token: ProcessStartToken,
}

/// Spawn failure without converting failure into a process record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct SpawnFailed {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Terminal incarnation that requested the process.
    pub terminal_incarnation: TerminalIncarnation,
    /// Stable machine-readable failure kind.
    pub kind: SpawnFailureKind,
    /// Typed cause of the failure.
    pub reason: SpawnFailureReason,
    /// Recovery guidance for the cause.
    pub hint: String,
    /// Operating-system error number, when the OS reported one.
    pub os_error: Option<i32>,
}

/// Typed spawn failures understood by the core.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SpawnFailureKind {
    /// Executable could not be started.
    Unavailable,
    /// Requested working directory or executable was invalid.
    InvalidRequest,
    /// Holder resource limits prevented creation.
    LimitExceeded,
}
