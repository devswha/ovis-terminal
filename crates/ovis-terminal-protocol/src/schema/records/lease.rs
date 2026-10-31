//! Lease and first-paint checkpoint records.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::common::Geometry;
use crate::schema::ids::{ClientId, LeaseId};
use crate::schema::scalar::{ByteOffset, Count, LeaseEpoch, OpenEnum, Revision};

/// Authority a lease grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LeaseRole {
    /// May write input and, unless released, owns geometry.
    Interact,
    /// Read-only; never writes input, resizes or takes over.
    Observe,
}

/// One lease on a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct LeaseRecord {
    /// Lease identity.
    pub lease_id: LeaseId,
    /// Lease epoch.
    pub epoch: LeaseEpoch,
    /// Granted role.
    pub role: OpenEnum<LeaseRole>,
    /// Client that holds the lease.
    pub holder: ClientId,
}

/// A terminal mode the VT engine tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalMode {
    /// Bracketed paste.
    BracketedPaste,
    /// Alternate screen.
    AltScreen,
    /// Mouse reporting.
    Mouse,
    /// Application cursor keys.
    AppCursor,
    /// Win32 input mode.
    Win32Input,
    /// Modify-other-keys reporting.
    ModifyOtherKeys,
}

/// Terminal modes a client must reproduce to paint correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Modes {
    /// Modes currently enabled.
    pub enabled: Vec<OpenEnum<TerminalMode>>,
    /// Kitty keyboard protocol flags in effect.
    pub kitty_keyboard: u8,
    /// Mode revision; bumps on every mode change.
    pub revision: Revision,
}

/// Cursor position and visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ScreenCursor {
    /// Zero-based row.
    pub row: u16,
    /// Zero-based column.
    pub col: u16,
    /// Whether the cursor is visible.
    pub visible: bool,
}

/// Offset-fenced checkpoint that precedes live bytes on a stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Checkpoint {
    /// Raw output offset the checkpoint represents.
    pub output_offset: ByteOffset,
    /// Filtered stream offset where the checkpoint bytes begin.
    pub stream_offset: ByteOffset,
    /// Screen revision.
    pub revision: Revision,
    /// Grid size.
    pub geometry: Geometry,
    /// Terminal modes.
    pub modes: Modes,
    /// Cursor state.
    pub cursor: ScreenCursor,
    /// Primary-screen history lines readable behind the paint: the history cursor.
    pub history_lines: Count,
}
