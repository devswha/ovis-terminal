//! Typed failure records carried by spawn, input and exit messages.

use serde::{Deserialize, Serialize};

use super::io::TerminalTarget;

/// Typed cause of a spawn that kept no terminal record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SpawnFailureReason {
    /// The argument vector named no program.
    EmptyArgv,
    /// A grid dimension was zero.
    InvalidGrid,
    /// The working directory could not be inspected.
    CwdUnavailable,
    /// The working directory is not a directory.
    CwdNotDirectory,
    /// The executable path could not be inspected.
    ExecutableUnavailable,
    /// The executable was not found on PATH, not executable, or a directory.
    ExecutableRejected,
    /// No pseudo-terminal could be allocated.
    PtyUnavailable,
    /// The pseudo-terminal master exposed no descriptor.
    PtyDescriptorMissing,
    /// The operating system failed to start the program.
    ExecFailed,
    /// The PTY library returned a child without exit-signal evidence.
    UnsupportedChildHandle,
    /// The started program's start token could not be read; it was stopped.
    StartTokenUnavailable,
    /// The holder is draining and accepts no new terminals.
    HolderDraining,
    /// A terminal worker thread could not be started; the program was stopped.
    WorkerUnavailable,
}

/// Typed cause of an input request that was not queued.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputRejection {
    /// The per-terminal input queue bound would be exceeded.
    InputOverflow,
}

/// Holder rejection of one input request before any of its bytes were queued.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct InputRejected {
    /// Terminal and process identity the input named.
    pub target: TerminalTarget,
    /// Rejected input sequence; it was not admitted and may be resent.
    #[serde(with = "crate::protocol::decimal")]
    pub sequence: u64,
    /// Typed rejection kind.
    pub reason: InputRejection,
    /// Recovery guidance for the rejection.
    pub hint: String,
    /// Bytes already waiting for the PTY.
    #[serde(with = "crate::protocol::decimal")]
    pub queued_bytes: u64,
    /// Entries already waiting for the PTY.
    #[serde(with = "crate::protocol::decimal")]
    pub queued_entries: u64,
    /// Per-terminal byte bound.
    #[serde(with = "crate::protocol::decimal")]
    pub limit_bytes: u64,
    /// Per-terminal entry bound.
    #[serde(with = "crate::protocol::decimal")]
    pub limit_entries: u64,
}

/// Why no exit status could be observed for a process.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExitUnknownReason {
    /// Waiting for the process failed.
    WaitFailed,
    /// The thread observing the process ended without reporting.
    WaiterLost,
}

/// Exit evidence that is unknown rather than a status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct ExitUnknown {
    /// Typed cause.
    pub reason: ExitUnknownReason,
    /// Operating-system error number, when the OS reported one.
    pub os_error: Option<i32>,
}

/// PTY operation that failed unexpectedly (not a terminal hang-up).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PtyOperation {
    /// Reading program output.
    Read,
    /// Waiting for the PTY to become readable or writable.
    Poll,
    /// Writing program input.
    Write,
    /// Recording output into the retained tail.
    Record,
    /// The output pump ended without reporting.
    Pump,
}

/// An unexpected PTY I/O failure; output may end before the program's last byte.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct PtyIoFailure {
    /// Failed operation.
    pub operation: PtyOperation,
    /// Operating-system error number, when the OS reported one.
    pub os_error: Option<i32>,
    /// Raw `poll(2)` revents bits, when poll reported error flags.
    pub poll_revents: Option<u16>,
}
