//! Offset-fenced checkpoint format and its versioned encoding.

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Line, Point};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Rgb};
use serde::{Deserialize, Serialize};

use crate::engine::error::{CheckpointVersion, VtError};
use crate::engine::fidelity::Fidelity;
use crate::engine::modes::ScreenBuffer;
use crate::engine::read::CursorShape as Shape;
use crate::engine::shadow::Shadow;
use crate::engine::tracker::StringKind;
use crate::engine::units::{ContentRevision, ModeRevision, OutputOffset, VtConfig};

/// Format name written into every checkpoint.
pub const CHECKPOINT_FORMAT: &str = "ovis-terminal-vt-checkpoint";
/// Format revision this engine writes and reads.
pub const CHECKPOINT_VERSION: u32 = 1;
/// Emulator whose grid serialization the checkpoint embeds; bump with the
/// `alacritty_terminal` dependency.
pub const CHECKPOINT_ENGINE: &str = "alacritty_terminal 0.26.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum CharsetRecord {
    Ascii,
    LineDrawing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct CursorRecord {
    pub(super) point: Point,
    pub(super) template: Cell,
    pub(super) charsets: [CharsetRecord; 4],
    pub(super) input_needs_wrap: bool,
}

/// One screen buffer: cells and history plus both cursors, which alacritty's
/// own grid serialization skips.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ScreenRecord {
    pub(super) grid: Grid<Cell>,
    pub(super) cursor: CursorRecord,
    pub(super) saved_cursor: CursorRecord,
}

/// Logical equality: alacritty's `Grid` equality asserts an unrotated ring
/// buffer and panics on any grid that has scrolled.
fn same_content(left: &Grid<Cell>, right: &Grid<Cell>) -> bool {
    left.columns() == right.columns()
        && left.screen_lines() == right.screen_lines()
        && left.history_size() == right.history_size()
        && left.display_offset() == right.display_offset()
        && (left.topmost_line().0..=left.bottommost_line().0)
            .all(|line| left[Line(line)] == right[Line(line)])
}

impl PartialEq for ScreenRecord {
    fn eq(&self, other: &Self) -> bool {
        self.cursor == other.cursor
            && self.saved_cursor == other.saved_cursor
            && same_content(&self.grid, &other.grid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CursorStyleRecord {
    pub(super) shape: Shape,
    pub(super) blinking: bool,
}

impl From<CursorStyle> for CursorStyleRecord {
    fn from(style: CursorStyle) -> Self {
        let shape = match style.shape {
            CursorShape::Block => Shape::Block,
            CursorShape::Underline => Shape::Underline,
            CursorShape::Beam => Shape::Beam,
            CursorShape::HollowBlock => Shape::HollowBlock,
            CursorShape::Hidden => Shape::Hidden,
        };
        Self {
            shape,
            blinking: style.blinking,
        }
    }
}

impl From<CursorStyleRecord> for CursorStyle {
    fn from(record: CursorStyleRecord) -> Self {
        let shape = match record.shape {
            Shape::Block => CursorShape::Block,
            Shape::Underline => CursorShape::Underline,
            Shape::Beam => CursorShape::Beam,
            Shape::HollowBlock => CursorShape::HollowBlock,
            Shape::Hidden => CursorShape::Hidden,
        };
        Self {
            shape,
            blinking: record.blinking,
        }
    }
}

/// Engine state at one output offset, plus the bytes of a sequence still open
/// there, so that importing it and feeding the bytes after the offset
/// reproduces an engine that saw the whole stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub(super) format: String,
    pub(super) version: u32,
    pub(super) engine: String,
    pub(super) offset: OutputOffset,
    pub(super) revision: ContentRevision,
    pub(super) mode_revision: ModeRevision,
    pub(super) config: VtConfig,
    pub(super) active: ScreenBuffer,
    pub(super) primary: ScreenRecord,
    pub(super) alternate: ScreenRecord,
    pub(super) mode_bits: u32,
    pub(super) colors: Vec<(u16, Rgb)>,
    pub(super) cursor_style: CursorStyleRecord,
    pub(super) shadow: Shadow,
    pub(super) fidelity: Fidelity,
    pub(super) pending: Vec<u8>,
    pub(super) draining: Option<StringKind>,
}

#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    engine: String,
}

impl Checkpoint {
    /// The identity this engine writes and accepts.
    pub fn supported_version() -> CheckpointVersion {
        CheckpointVersion {
            format: CHECKPOINT_FORMAT.to_owned(),
            version: CHECKPOINT_VERSION,
            engine: CHECKPOINT_ENGINE.to_owned(),
        }
    }

    /// Output offset the checkpoint describes.
    pub const fn offset(&self) -> OutputOffset {
        self.offset
    }

    /// Content revision at the offset.
    pub const fn revision(&self) -> ContentRevision {
        self.revision
    }

    /// Whether import reproduces the stream exactly.
    pub const fn fidelity(&self) -> &Fidelity {
        &self.fidelity
    }

    /// Bytes of the sequence open at the offset, replayed on import.
    pub fn pending_bytes(&self) -> &[u8] {
        &self.pending
    }

    /// Encodes the checkpoint as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::CheckpointEncoding`] when serialization fails.
    pub fn to_bytes(&self) -> Result<Vec<u8>, VtError> {
        serde_json::to_vec(self).map_err(VtError::CheckpointEncoding)
    }

    /// Decodes a checkpoint, checking its format identity first.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::CheckpointIncompatible`] for another format revision
    /// or emulator, and [`VtError::CheckpointMalformed`] for undecodable bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VtError> {
        let envelope: Envelope =
            serde_json::from_slice(bytes).map_err(VtError::CheckpointMalformed)?;
        check_identity(CheckpointVersion {
            format: envelope.format,
            version: envelope.version,
            engine: envelope.engine,
        })?;
        serde_json::from_slice(bytes).map_err(VtError::CheckpointMalformed)
    }
}

/// Rejects any format identity other than the one this engine writes.
pub(super) fn check_identity(found: CheckpointVersion) -> Result<(), VtError> {
    let supported = Checkpoint::supported_version();
    if found == supported {
        Ok(())
    } else {
        Err(VtError::CheckpointIncompatible {
            found: Box::new(found),
            supported: Box::new(supported),
        })
    }
}
