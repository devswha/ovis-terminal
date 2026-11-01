//! The observation protocol between the harness and the control process: one JSON
//! object per line over the control process's Unix socket. Sequences, offsets, byte and
//! item counts are canonical decimal strings.

use ovis_terminal_holder::protocol::{ProcessStartToken, TerminalTarget};
use ovis_terminal_protocol::domain::recovery::{CrashRecovery, TerminalRecovery};
use ovis_terminal_protocol::domain::restart::InterruptedOp;
use ovis_terminal_protocol::schema::ids::ProcessPid;
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, Count, EventSeq};
use serde::{Deserialize, Serialize};

/// The first line a viewer sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hello {
    pub(super) viewer: String,
    /// `false` for a viewer that never reads: a stalled viewer.
    pub(super) reads: bool,
    /// Resume after an interruption at this sequence; absent replays what is retained.
    #[serde(default)]
    pub(super) from_seq: Option<EventSeq>,
}

/// A viewer request after its hello.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Ping { id: Count },
    Status { id: Count },
}

/// One line the control process writes to a viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Line {
    /// A published event of this control boot.
    Event {
        seq: EventSeq,
        event: Event,
    },
    /// Events `[from, to)` are no longer retained for replay.
    Gap {
        from: EventSeq,
        to: EventSeq,
    },
    Pong {
        id: Count,
    },
    Status {
        id: Count,
        health: Health,
    },
}

/// What the control process publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Event {
    Ready(Box<Ready>),
    /// Stream head of every terminal, by index; `null` once a terminal is gone.
    Progress {
        offsets: Vec<Option<ByteOffset>>,
        checkpoints: Count,
    },
    Checkpointed {
        terminal: Count,
        offset: ByteOffset,
    },
    Exited {
        terminal: Count,
        final_offset: ByteOffset,
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// A viewer whose bounded queue overflowed was disconnected.
    ViewerCut {
        viewer: String,
        queued_bytes: ByteCount,
        dropped: Count,
    },
    /// Input whose intent is durable was handed to the holder.
    InputSent {
        terminal: Count,
        op_id: String,
        sequence: Count,
    },
    HandedOver {
        offsets: Vec<Option<ByteOffset>>,
    },
    LinkLost {
        group: Count,
    },
}

/// How this control boot began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(super) enum BootMode {
    /// First boot: spawn every program.
    Fresh,
    /// After SIGKILL: crash recovery.
    Crash,
    /// After a planned handover.
    Handover,
}

/// The first event of every control boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ready {
    pub(super) mode: BootMode,
    pub(super) boot: Count,
    pub(super) terminals: Vec<Identity>,
    pub(super) recovery: Option<Recovery>,
    pub(super) interrupted: Vec<InterruptedOp>,
}

/// A recorded program and what its holder reported at this boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub(super) index: Count,
    pub(super) target: TerminalTarget,
    pub(super) group: Count,
    pub(super) pid: ProcessPid,
    pub(super) start_token: ProcessStartToken,
    /// Holder input sequence at adoption; absent when the holder no longer holds it.
    pub(super) input_sequence: Option<Count>,
}

/// What a restart proved about each recorded program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Recovery {
    Crash(Box<CrashRecovery>),
    Handover { terminals: Vec<TerminalRecovery> },
}

impl Recovery {
    pub(super) fn terminals(&self) -> &[TerminalRecovery] {
        match self {
            Self::Crash(report) => &report.terminals,
            Self::Handover { terminals } => terminals,
        }
    }
}

/// Read-only health of the control process: streams, queues and lag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Health {
    pub(super) head: EventSeq,
    pub(super) queue_limit_bytes: ByteCount,
    pub(super) viewers: Vec<ViewerHealth>,
    /// Output bytes no retained source holds, per terminal.
    pub(super) lag: Vec<ByteCount>,
    pub(super) accept_failures: Count,
}

/// One viewer's queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ViewerHealth {
    pub(super) viewer: String,
    pub(super) reads: bool,
    pub(super) queued_bytes: ByteCount,
    pub(super) max_queued_bytes: ByteCount,
    pub(super) sent: Count,
    pub(super) cut: bool,
    pub(super) failures: Count,
}
