use std::fmt::{Display, Formatter};

/// A malformed or incomplete wire record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameError {
    /// A frame payload exceeds the protocol limit.
    FrameTooLarge {
        /// Declared payload size.
        length: usize,
        /// Maximum permitted payload.
        limit: usize,
        /// Offset where the header begins.
        offset: u64,
    },
    /// A frame kind is not part of the protocol.
    UnknownKind {
        /// Unrecognized wire value.
        kind: u8,
        /// Offset of the frame header.
        offset: u64,
    },
    /// The stream ended with an incomplete frame header.
    TruncatedHeader {
        /// Offset where the header begins.
        offset: u64,
        /// Number of header bytes received.
        actual: usize,
    },
    /// The stream ended before the declared frame payload was complete.
    TruncatedPayload {
        /// Offset where the frame begins.
        offset: u64,
        /// Number of payload bytes declared.
        expected: usize,
        /// Number of payload bytes received.
        actual: usize,
    },
    /// A control record is longer than the NDJSON limit.
    ControlLineTooLarge {
        /// Observed byte length, including a newline when present.
        length: usize,
        /// Maximum permitted control-record size.
        limit: usize,
        /// Offset where the control record begins.
        offset: u64,
    },
    /// A transport read exceeds the decoder's bounded pending-input window.
    BufferedInputTooLarge {
        /// Pending and incoming bytes combined.
        length: usize,
        /// Maximum accepted pending-input window.
        limit: usize,
        /// Offset where the pending input begins.
        offset: u64,
    },
    /// A control record contains non-UTF-8 bytes.
    InvalidControlUtf8 {
        /// Offset where the control record begins.
        offset: u64,
    },
    /// The decoder has already crossed the hello upgrade boundary.
    UpgradeAlreadyPerformed {
        /// Current wire offset.
        offset: u64,
    },
    /// A byte offset cannot be represented without loss.
    OffsetOverflow {
        /// Offset before the failed addition.
        offset: u64,
        /// Number of bytes being added.
        amount: usize,
    },
}

impl Display for FrameError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FrameTooLarge {
                length,
                limit,
                offset,
            } => write!(formatter, "frame_too_large: {length} > {limit} at {offset}"),
            Self::UnknownKind { kind, offset } => {
                write!(formatter, "unknown_frame_kind: {kind} at {offset}")
            }
            Self::TruncatedHeader { offset, actual } => {
                write!(
                    formatter,
                    "truncated_frame_header: {actual} bytes at {offset}"
                )
            }
            Self::TruncatedPayload {
                offset,
                expected,
                actual,
            } => write!(
                formatter,
                "truncated_frame_payload: {actual}/{expected} bytes at {offset}"
            ),
            Self::ControlLineTooLarge {
                length,
                limit,
                offset,
            } => write!(
                formatter,
                "control_line_too_large: {length} > {limit} at {offset}"
            ),
            Self::BufferedInputTooLarge {
                length,
                limit,
                offset,
            } => write!(
                formatter,
                "buffered_input_too_large: {length} > {limit} at {offset}"
            ),
            Self::InvalidControlUtf8 { offset } => {
                write!(formatter, "invalid_control_utf8 at {offset}")
            }
            Self::UpgradeAlreadyPerformed { offset } => {
                write!(formatter, "hello_upgrade_already_performed at {offset}")
            }
            Self::OffsetOverflow { offset, amount } => {
                write!(formatter, "byte_offset_overflow: {offset} + {amount}")
            }
        }
    }
}

impl std::error::Error for FrameError {}
