use std::str::Utf8Error;

use thiserror::Error;

use super::WireOffset;

/// A malformed wire record or bounded-decoder failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameError {
    /// A frame payload exceeds the protocol limit.
    #[error("frame_too_large: {length} > {limit} at {offset}")]
    FrameTooLarge {
        /// Declared payload size.
        length: usize,
        /// Maximum permitted payload.
        limit: usize,
        /// Offset where the header begins.
        offset: WireOffset,
    },
    /// A frame kind is not part of the protocol.
    #[error("unknown_kind: {kind} at {offset}")]
    UnknownKind {
        /// Unrecognized wire value.
        kind: u8,
        /// Offset of the frame header.
        offset: WireOffset,
    },
    /// The stream ended with an incomplete frame header.
    #[error("truncated_frame_header: {actual} bytes at {offset}")]
    TruncatedHeader {
        /// Offset where the header begins.
        offset: WireOffset,
        /// Number of header bytes received.
        actual: usize,
    },
    /// The stream ended before the declared frame payload was complete.
    #[error("truncated_frame_payload: {actual}/{expected} bytes at {offset}")]
    TruncatedPayload {
        /// Offset where the frame begins.
        offset: WireOffset,
        /// Number of payload bytes declared.
        expected: usize,
        /// Number of payload bytes received.
        actual: usize,
    },
    /// A control record is longer than the NDJSON limit.
    #[error("control_line_too_large: {length} > {limit} at {offset}")]
    ControlLineTooLarge {
        /// Observed byte length, including a newline when present.
        length: usize,
        /// Maximum permitted control-record size.
        limit: usize,
        /// Offset where the control record begins.
        offset: WireOffset,
    },
    /// A transport read exceeds the decoder's bounded pending-input window.
    #[error("buffered_input_too_large: {length} > {limit} at {offset}")]
    BufferedInputTooLarge {
        /// Pending and incoming bytes combined.
        length: usize,
        /// Maximum accepted pending-input window.
        limit: usize,
        /// Offset where the pending input begins.
        offset: WireOffset,
    },
    /// A control record contains non-UTF-8 bytes.
    #[error("invalid_control_utf8 at {offset}")]
    InvalidControlUtf8 {
        /// Offset where the control record begins.
        offset: WireOffset,
        /// Original UTF-8 validation failure.
        #[source]
        source: Utf8Error,
    },
    /// The decoder has already crossed the hello upgrade boundary.
    #[error("hello_upgrade_already_performed at {offset}")]
    UpgradeAlreadyPerformed {
        /// Current wire offset.
        offset: WireOffset,
    },
    /// A byte offset cannot be represented without loss.
    #[error("byte_offset_overflow: {offset} + {amount}")]
    OffsetOverflow {
        /// Offset before the failed addition.
        offset: WireOffset,
        /// Number of bytes being added.
        amount: usize,
    },
    /// A logical result length cannot be represented as a wire total.
    #[error("logical_length_overflow: {length}")]
    LogicalLengthOverflow {
        /// Logical byte length that could not be represented.
        length: usize,
    },
    /// Internal queue accounting could not be represented safely.
    #[error("queue_accounting_overflow")]
    QueueAccountingOverflow,
}

impl FrameError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::FrameTooLarge { .. } => "frame_too_large",
            Self::UnknownKind { .. } => "unknown_kind",
            Self::TruncatedHeader { .. } | Self::TruncatedPayload { .. } => "truncated_frame",
            Self::ControlLineTooLarge { .. } => "control_line_too_large",
            Self::BufferedInputTooLarge { .. } => "buffered_input_too_large",
            Self::InvalidControlUtf8 { .. } => "invalid_control_utf8",
            Self::UpgradeAlreadyPerformed { .. } => "hello_upgrade_already_performed",
            Self::OffsetOverflow { .. } => "offset_overflow",
            Self::LogicalLengthOverflow { .. } => "logical_length_overflow",
            Self::QueueAccountingOverflow => "queue_accounting_overflow",
        }
    }

    /// Returns a stable recovery hint for the machine-readable error kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::FrameTooLarge { .. } => "reduce the frame payload to the advertised limit",
            Self::UnknownKind { .. } => "use a frame kind supported by this protocol generation",
            Self::TruncatedHeader { .. } | Self::TruncatedPayload { .. } => {
                "continue the same byte stream or report transport truncation"
            }
            Self::ControlLineTooLarge { .. } => "reduce the NDJSON control record size",
            Self::BufferedInputTooLarge { .. } => "drain decoded records before feeding more bytes",
            Self::InvalidControlUtf8 { .. } => "send a UTF-8 NDJSON control record",
            Self::UpgradeAlreadyPerformed { .. } => {
                "create a new connection for another hello upgrade"
            }
            Self::OffsetOverflow { .. } | Self::LogicalLengthOverflow { .. } => {
                "use a representable byte range"
            }
            Self::QueueAccountingOverflow => "recreate the connection decoder",
        }
    }
}
