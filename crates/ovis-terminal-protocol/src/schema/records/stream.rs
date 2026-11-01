//! Plain-mode stream notifications: `attach.control` records and `attach.data` bytes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::graphics::GraphicsDroppedData;
use crate::schema::events::StreamCloseReason;
use crate::schema::ids::StreamId;
use crate::schema::records::{Checkpoint, LeaseRecord};
use crate::schema::scalar::{ByteOffset, OpenEnum};

/// Kind of an `attach.control` record; control never travels as terminal bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlKind {
    /// A checkpoint paint begins at `stream_offset`.
    Checkpoint,
    /// Bytes were skipped; a checkpoint follows.
    Gap,
    /// The interact stream is ready for input.
    InputReady,
    /// Input readiness was revoked.
    InputRevoked,
    /// The stream closed; no record or byte follows.
    Close,
    /// A whole graphics sequence over its cap was dropped where this record stands.
    GraphicsDropped,
}

/// Raw output the stream skipped when it was resynchronized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SkippedOutput {
    /// First raw output offset not delivered.
    pub from_output_offset: ByteOffset,
    /// Raw output offset the following checkpoint represents.
    pub to_output_offset: ByteOffset,
}

impl SkippedOutput {
    /// Raw output from `from_output_offset` up to `to_output_offset` was not delivered.
    #[must_use]
    pub const fn new(from_output_offset: ByteOffset, to_output_offset: ByteOffset) -> Self {
        Self {
            from_output_offset,
            to_output_offset,
        }
    }
}

/// Why and where a stream closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct StreamClose {
    /// Reason.
    pub reason: OpenEnum<StreamCloseReason>,
    /// Final filtered stream offset; every byte before it was delivered.
    pub final_offset: ByteOffset,
    /// Raw output offset of the final byte, when the program exited.
    pub final_output_offset: Option<ByteOffset>,
    /// Exit code, when the program exited with one.
    pub exit_code: Option<i32>,
    /// Signal number, when a signal ended the program.
    pub signal: Option<i32>,
    /// Lease that displaced this stream's lease.
    pub by: Option<LeaseRecord>,
}

impl StreamClose {
    /// A close for `reason` after `final_offset` delivered bytes.
    #[must_use]
    pub const fn new(reason: StreamCloseReason, final_offset: ByteOffset) -> Self {
        Self {
            reason: OpenEnum::Known(reason),
            final_offset,
            final_output_offset: None,
            exit_code: None,
            signal: None,
            by: None,
        }
    }
}

/// Params of the `attach.control` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachControl {
    /// Stream.
    pub stream_id: StreamId,
    /// Record kind.
    pub kind: OpenEnum<ControlKind>,
    /// Filtered stream offset the record applies at.
    pub stream_offset: ByteOffset,
    /// Checkpoint, for `checkpoint`.
    pub checkpoint: Option<Checkpoint>,
    /// Skipped raw output, for `gap`.
    pub skipped: Option<SkippedOutput>,
    /// Close details, for `close`.
    pub close: Option<StreamClose>,
    /// The dropped sequence, for `graphics_dropped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_dropped: Option<GraphicsDroppedData>,
}

impl AttachControl {
    /// A record of `kind` at `stream_offset` with no detail.
    #[must_use]
    pub const fn new(stream_id: StreamId, kind: ControlKind, stream_offset: ByteOffset) -> Self {
        Self {
            stream_id,
            kind: OpenEnum::Known(kind),
            stream_offset,
            checkpoint: None,
            skipped: None,
            close: None,
            graphics_dropped: None,
        }
    }
}

/// Params of the `attach.data` notification: terminal bytes only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttachData {
    /// Stream.
    pub stream_id: StreamId,
    /// Filtered stream offset of the first byte.
    pub offset: ByteOffset,
    /// Base64 terminal bytes, at most one stream chunk decoded.
    pub data_b64: String,
}

impl AttachData {
    /// Bytes `data_b64` of `stream_id` beginning at `offset`.
    #[must_use]
    pub const fn new(stream_id: StreamId, offset: ByteOffset, data_b64: String) -> Self {
        Self {
            stream_id,
            offset,
            data_b64,
        }
    }
}
