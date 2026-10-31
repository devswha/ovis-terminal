//! Pure-Rust VT screen state, history and terminal-mode tracking for ovis-terminal.
//!
//! [`Vt`] wraps `alacritty_terminal` as the authoritative image of one
//! terminal's output stream: screen, scrollback, cursor and structured modes,
//! offset-fenced checkpoints whose exactness covers the parser boundary, query
//! replies located in the stream, and bounded OSC records for the agent owner.

mod engine;
mod shard;

pub use engine::{
    CHECKPOINT_ENGINE, CHECKPOINT_FORMAT, CHECKPOINT_VERSION, Checkpoint, CheckpointVersion,
    ClientFact, ContentRevision, CursorShape, CursorState, EditingModes, FeedOutcome, Fidelity,
    FirstPaint, Geometry, HistoryCompleteness, HistoryLimit, HistoryRead, HistoryUnavailableReason,
    InexactReason, KeyEncoding, KeyboardModes, LineSpan, MIN_COLUMNS, ModeRevision,
    ModifyOtherKeys, MouseEncoding, MouseModes, MouseTracking, OSC_FIELD_LIMIT, OSC_RECORD_LIMIT,
    OscContent, OscFault, OscRecord, OscTerminator, OutputOffset, PasteMode, QueryOutcome,
    ReadFormat, ReadSource, ScreenBuffer, ScreenLine, ScreenModes, ScreenRead, SearchMatch,
    TerminalModes, TerminalQuery, Vt, VtConfig, VtError,
};
pub use shard::{ParseBudget, PendingCap, Shard, ShardError, Turn};

/// Cargo package name of this crate, used by the workspace layering record.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
pub const DEPENDS_ON: &[&str] = &[ovis_terminal_protocol::CRATE_NAME];
