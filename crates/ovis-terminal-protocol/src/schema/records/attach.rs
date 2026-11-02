//! Attach outcome details and the plain-mode stream notifications.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::common::Geometry;
use crate::schema::ids::{ClientId, LeaseId, StreamId, TerminalId};
use crate::schema::methods::attach::{AttachOpenResult, AttachOutcome, AttachStreamResult};
use crate::schema::records::{
    Checkpoint, CheckpointInexact, LeaseRecord, LeaseRole, Modes, ScreenCursor, TerminalMode,
};
use crate::schema::scalar::{
    ByteCount, ByteOffset, Count, Incarnation, LeaseEpoch, OpenEnum, Revision,
};

/// The state change that lets a held or recovering attach succeed on retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetryOn {
    /// The named interact lease is released or revoked (`lease.revoked`).
    LeaseReleased,
    /// The terminal finishes recovery (`server.recovery`).
    Recovered,
}

/// An actionable retry boundary: which event to wait for, never a blind poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RetryBoundary {
    /// The state change to wait for.
    pub on: OpenEnum<RetryOn>,
    /// Event type that announces it.
    pub event: String,
    /// Lease whose release ends the hold.
    pub lease: Option<LeaseRecord>,
}

impl RetryBoundary {
    /// Retry once `holder` is released or revoked.
    #[must_use]
    pub fn lease_released(holder: LeaseRecord) -> Self {
        Self {
            on: OpenEnum::Known(RetryOn::LeaseReleased),
            event: "lease.revoked".to_owned(),
            lease: Some(holder),
        }
    }

    /// Retry once the terminal's recovery is announced.
    #[must_use]
    pub fn recovered() -> Self {
        Self {
            on: OpenEnum::Known(RetryOn::Recovered),
            event: "server.recovery".to_owned(),
            lease: None,
        }
    }
}

/// The terminal and program incarnations a stream is bound to; never inherited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachedTerminal {
    /// Terminal identity.
    pub terminal_id: TerminalId,
    /// Terminal incarnation.
    pub terminal_incarnation: Incarnation,
    /// Program incarnation, when a program is running.
    pub process_incarnation: Option<Incarnation>,
}

impl AttachedTerminal {
    /// Names `terminal_id` at `terminal_incarnation` running `process_incarnation`.
    #[must_use]
    pub const fn new(
        terminal_id: TerminalId,
        terminal_incarnation: Incarnation,
        process_incarnation: Option<Incarnation>,
    ) -> Self {
        Self {
            terminal_id,
            terminal_incarnation,
            process_incarnation,
        }
    }
}

impl AttachOpenResult {
    /// An outcome with no stream, lease or paint yet.
    #[must_use]
    pub const fn new(outcome: AttachOutcome, credit_initial: ByteCount) -> Self {
        Self {
            outcome: OpenEnum::Known(outcome),
            stream_id: None,
            channel: None,
            lease: None,
            holder: None,
            displaced: None,
            first_paint: None,
            credit_initial,
            terminal: None,
            retry: None,
            reason: None,
            final_output_offset: None,
        }
    }
}

impl Checkpoint {
    /// A checkpoint at raw `output_offset`, beginning at filtered `stream_offset`.
    #[must_use]
    pub fn new(
        output_offset: ByteOffset,
        stream_offset: ByteOffset,
        screen: PaintedScreen,
    ) -> Self {
        Self {
            output_offset,
            stream_offset,
            revision: screen.revision,
            geometry: screen.geometry,
            modes: screen.modes,
            cursor: screen.cursor,
            history_lines: screen.history_lines,
            inexact: screen.inexact,
        }
    }
}

/// The screen facts a checkpoint carries besides its two offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PaintedScreen {
    /// Screen revision.
    pub revision: Revision,
    /// Grid size.
    pub geometry: Geometry,
    /// Terminal modes.
    pub modes: Modes,
    /// Cursor state.
    pub cursor: ScreenCursor,
    /// Primary-screen history lines readable behind the paint.
    pub history_lines: Count,
    /// Why the paint is inexact; empty when exact.
    pub inexact: Vec<OpenEnum<CheckpointInexact>>,
}

impl PaintedScreen {
    /// Screen facts at `revision` on `geometry`, with default modes and a home cursor.
    #[must_use]
    pub const fn new(revision: Revision, geometry: Geometry) -> Self {
        Self {
            revision,
            geometry,
            modes: Modes {
                enabled: Vec::new(),
                kitty_keyboard: 0,
                revision: Revision::new(0),
            },
            cursor: ScreenCursor {
                row: 0,
                col: 0,
                visible: true,
            },
            history_lines: Count::new(0),
            inexact: Vec::new(),
        }
    }
}

impl Modes {
    /// Modes `enabled` with kitty keyboard flags at mode `revision`.
    #[must_use]
    pub const fn new(
        enabled: Vec<OpenEnum<TerminalMode>>,
        kitty_keyboard: u8,
        revision: Revision,
    ) -> Self {
        Self {
            enabled,
            kitty_keyboard,
            revision,
        }
    }
}

impl ScreenCursor {
    /// A cursor at zero-based `row` and `col`.
    #[must_use]
    pub const fn new(row: u16, col: u16, visible: bool) -> Self {
        Self { row, col, visible }
    }
}

impl LeaseRecord {
    /// Lease `lease_id` at `epoch` held by `holder` with `role`.
    #[must_use]
    pub fn new(
        lease_id: LeaseId,
        epoch: LeaseEpoch,
        (role, holder): (LeaseRole, ClientId),
    ) -> Self {
        Self {
            lease_id,
            epoch,
            role: OpenEnum::Known(role),
            holder,
        }
    }
}

impl AttachStreamResult {
    /// Stream `stream_id` at filtered offset `stream_offset`.
    #[must_use]
    pub const fn new(
        stream_id: StreamId,
        stream_offset: ByteOffset,
        checkpoint: Option<Checkpoint>,
    ) -> Self {
        Self {
            stream_id,
            stream_offset,
            checkpoint,
        }
    }
}
