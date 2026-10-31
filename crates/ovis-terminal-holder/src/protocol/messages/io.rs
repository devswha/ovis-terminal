//! Terminal byte, input, geometry, and process-control messages.

use serde::{Deserialize, Serialize};

use super::outcome::{ExitUnknown, PtyIoFailure};
use crate::protocol::{ProcessIncarnation, TerminalId, TerminalIncarnation};

/// Contiguous raw PTY output bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Output {
    /// Stable terminal identity and incarnation.
    pub target: TerminalTarget,
    /// Offset of the first byte in `bytes`.
    #[serde(with = "crate::protocol::decimal")]
    pub offset: u64,
    /// Raw bytes, not decoded text.
    pub bytes: Vec<u8>,
}

/// Terminal identity tuple used in every target-sensitive holder message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct TerminalTarget {
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
    /// Terminal incarnation.
    pub terminal_incarnation: TerminalIncarnation,
    /// Process incarnation when the operation is process-specific.
    pub process_incarnation: Option<ProcessIncarnation>,
}

/// Authority proof rechecked immediately before queued bytes are written.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct WriteGrant {
    /// Grant epoch supplied by the authority owner.
    #[serde(with = "crate::protocol::decimal")]
    pub grant_epoch: u64,
    /// Fence epoch observed when the core admitted the input.
    #[serde(with = "crate::protocol::decimal")]
    pub fence_epoch: u64,
    /// Process incarnation the grant is bound to.
    pub process_incarnation: ProcessIncarnation,
}

/// Ordered input request, never an anonymous stream write.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Input {
    /// Terminal and process identity receiving the input.
    pub target: TerminalTarget,
    /// Per-terminal monotonically increasing input sequence.
    #[serde(with = "crate::protocol::decimal")]
    pub sequence: u64,
    /// Authority checked again at the actual write boundary.
    pub grant: WriteGrant,
    /// Raw input bytes.
    pub bytes: Vec<u8>,
}

/// Result of the actual PTY write boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct InputWritten {
    /// Terminal and process identity that received the input.
    pub target: TerminalTarget,
    /// Input sequence acknowledged by the holder.
    #[serde(with = "crate::protocol::decimal")]
    pub sequence: u64,
    /// Grant epoch rechecked at write time.
    #[serde(with = "crate::protocol::decimal")]
    pub grant_epoch: u64,
    /// Fence epoch applied at write time.
    #[serde(with = "crate::protocol::decimal")]
    pub fence_epoch: u64,
    /// Monotonic holder acknowledgement epoch.
    #[serde(with = "crate::protocol::decimal")]
    pub ack_epoch: u64,
    /// Number of bytes written, including a partial prefix.
    #[serde(with = "crate::protocol::decimal")]
    pub written_bytes: u64,
    /// Cumulative offset after the written prefix.
    #[serde(with = "crate::protocol::decimal")]
    pub output_offset: u64,
}

/// Resize request scoped to a terminal incarnation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Resize {
    /// Target terminal.
    pub target: TerminalTarget,
    /// New geometry as columns, rows.
    pub geometry: (u16, u16),
    /// Caller-owned geometry epoch.
    #[serde(with = "crate::protocol::decimal")]
    pub geometry_epoch: u64,
}

/// Signal request scoped to one process incarnation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Signal {
    /// Target terminal and process.
    pub target: TerminalTarget,
    /// Platform-neutral signal name.
    pub signal: SignalKind,
}

/// Supported process signal requests.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SignalKind {
    /// Request graceful termination.
    Interrupt,
    /// Request termination.
    Terminate,
    /// Force termination.
    Kill,
}

/// Process exit evidence; output through `final_offset` must precede delivery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Exit {
    /// Target process identity.
    pub target: TerminalTarget,
    /// Optional process exit code.
    pub code: Option<i32>,
    /// Optional platform signal number.
    pub signal: Option<i32>,
    /// Why no status was observed; `code` and `signal` are then absent.
    pub exit_unknown: Option<ExitUnknown>,
    /// Unexpected PTY failure that may have ended output early.
    pub io_failure: Option<PtyIoFailure>,
    /// Last output byte offset produced before exit.
    #[serde(with = "crate::protocol::decimal")]
    pub final_offset: u64,
}

/// Retained raw output range returned during adoption.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Tail {
    /// Target terminal identity.
    pub target: TerminalTarget,
    /// First raw output byte included.
    #[serde(with = "crate::protocol::decimal")]
    pub from_offset: u64,
    /// Exclusive offset immediately after the returned bytes.
    #[serde(with = "crate::protocol::decimal")]
    pub through_offset: u64,
    /// Retained raw output bytes.
    pub bytes: Vec<u8>,
    /// True when retention no longer covers the requested offset.
    pub gap: bool,
}

/// Core request for retained raw output starting at an offset.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct TailRequest {
    /// Target terminal identity.
    pub target: TerminalTarget,
    /// Requested first byte offset.
    #[serde(with = "crate::protocol::decimal")]
    pub from_offset: u64,
}

/// Acknowledgement that a retained terminal record can be forgotten.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::exhaustive_structs,
    reason = "adding fields to a frozen v1 message requires a generation change"
)]
pub struct Forget {
    /// Terminal and incarnation to remove.
    pub target: TerminalTarget,
    /// Exit offset acknowledged after all output through it was received.
    #[serde(default, with = "crate::protocol::optional_decimal")]
    pub acknowledged_final_offset: Option<u64>,
}
