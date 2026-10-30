use std::collections::VecDeque;

use super::decoder::DecodeLimits;
use super::{
    Frame, FrameDecoder, FrameError, MAX_FRAME_PAYLOAD, MAX_READY_BYTES, MAX_READY_RECORDS,
    WireOffset,
};

/// A complete message in either persistent NDJSON or upgraded framed mode.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireMessage {
    /// One UTF-8 NDJSON record without its terminating newline.
    ControlLine(Vec<u8>),
    /// One typed channel frame.
    Frame(Frame),
}

/// Result of feeding bytes to a connection decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FeedOutcome {
    /// All supplied bytes were accepted by the decoder.
    Accepted {
        /// Number of input bytes consumed.
        consumed: usize,
    },
    /// Drain ready messages, then resubmit input starting at `consumed`.
    NeedDrain {
        /// Number of input bytes consumed before backpressure.
        consumed: usize,
    },
}

/// Persistent connection decoder with an explicit hello upgrade boundary.
#[derive(Debug, Default)]
pub struct ConnectionDecoder {
    pending: Vec<u8>,
    ready: VecDeque<WireMessage>,
    ready_bytes: usize,
    frame_decoder: Option<FrameDecoder>,
    plain_offset: WireOffset,
}

impl ConnectionDecoder {
    /// Creates a decoder in persistent NDJSON control mode.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffers a transport read without treating read boundaries as records.
    ///
    /// In framed mode, callers must drain messages and resubmit any unconsumed suffix
    /// when [`FeedOutcome::NeedDrain`] is returned.
    ///
    /// # Errors
    /// Returns a typed error if pending plain-mode bytes exceed their bound.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<FeedOutcome, FrameError> {
        if self.frame_decoder.is_some() {
            self.pump_pending()?;
            if !self.pending.is_empty() {
                return Ok(FeedOutcome::NeedDrain { consumed: 0 });
            }
            return self.feed_framed(bytes);
        }
        let maximum = MAX_FRAME_PAYLOAD
            .checked_mul(2)
            .and_then(|size| size.checked_add(9))
            .ok_or(FrameError::OffsetOverflow {
                offset: self.plain_offset,
                amount: bytes.len(),
            })?;
        let combined =
            self.pending
                .len()
                .checked_add(bytes.len())
                .ok_or(FrameError::OffsetOverflow {
                    offset: self.plain_offset,
                    amount: bytes.len(),
                })?;
        if combined > maximum {
            return Err(FrameError::BufferedInputTooLarge {
                length: combined,
                limit: maximum,
                offset: self.plain_offset,
            });
        }
        self.pending.extend_from_slice(bytes);
        Ok(FeedOutcome::Accepted {
            consumed: bytes.len(),
        })
    }

    /// Returns the next complete line or frame, if one is available.
    ///
    /// # Errors
    /// Returns a typed error for invalid UTF-8, an overlong line, or malformed frame bytes.
    pub fn next_message(&mut self) -> Result<Option<WireMessage>, FrameError> {
        if self.frame_decoder.is_some() {
            self.pump_pending()?;
            let message = self.pop_ready()?;
            if message.is_some() {
                self.pump_pending()?;
            }
            return Ok(message);
        }
        let Some(line_end) = self.pending.iter().position(|byte| *byte == b'\n') else {
            if self.pending.len() > MAX_FRAME_PAYLOAD {
                return Err(FrameError::ControlLineTooLarge {
                    length: self.pending.len(),
                    limit: MAX_FRAME_PAYLOAD,
                    offset: self.plain_offset,
                });
            }
            return Ok(None);
        };
        if line_end > MAX_FRAME_PAYLOAD {
            return Err(FrameError::ControlLineTooLarge {
                length: line_end,
                limit: MAX_FRAME_PAYLOAD,
                offset: self.plain_offset,
            });
        }
        let line_length = line_end.checked_add(1).ok_or(FrameError::OffsetOverflow {
            offset: self.plain_offset,
            amount: line_end,
        })?;
        if line_length > MAX_FRAME_PAYLOAD {
            return Err(FrameError::ControlLineTooLarge {
                length: line_length,
                limit: MAX_FRAME_PAYLOAD,
                offset: self.plain_offset,
            });
        }
        let line_bytes = self
            .pending
            .get(..line_end)
            .ok_or(FrameError::QueueAccountingOverflow)?;
        std::str::from_utf8(line_bytes).map_err(|source| FrameError::InvalidControlUtf8 {
            offset: self.plain_offset,
            source,
        })?;
        let next_offset = self.plain_offset.checked_add(line_length)?;
        let line = self.pending.drain(..line_end).collect::<Vec<_>>();
        self.pending.drain(..1);
        self.plain_offset = next_offset;
        Ok(Some(WireMessage::ControlLine(line)))
    }

    /// Switches the parser after the successful plain-mode hello reply.
    ///
    /// Any buffered bytes after that line become framed bytes; this is the explicit
    /// protocol boundary and does not depend on transport read segmentation.
    ///
    /// # Errors
    /// Returns [`FrameError::UpgradeAlreadyPerformed`] when called more than once.
    pub fn upgrade_to_framed(&mut self) -> Result<(), FrameError> {
        if self.frame_decoder.is_some() {
            return Err(FrameError::UpgradeAlreadyPerformed {
                offset: self.plain_offset,
            });
        }
        self.frame_decoder = Some(FrameDecoder::new());
        self.pump_pending()
    }

    /// Returns the number of messages retained for the caller.
    #[must_use]
    pub fn queued_records(&self) -> usize {
        self.ready.len()
    }

    /// Returns the payload bytes retained in queued messages.
    #[must_use]
    pub const fn queued_bytes(&self) -> usize {
        self.ready_bytes
    }

    fn feed_framed(&mut self, bytes: &[u8]) -> Result<FeedOutcome, FrameError> {
        let limits = self.available_limits()?;
        let batch = self
            .frame_decoder
            .as_mut()
            .ok_or(FrameError::QueueAccountingOverflow)?
            .push_bounded(bytes, limits)?;
        let consumed = batch.consumed;
        self.enqueue_frames(batch.frames)?;
        if consumed < bytes.len() {
            Ok(FeedOutcome::NeedDrain { consumed })
        } else {
            Ok(FeedOutcome::Accepted { consumed })
        }
    }

    fn pump_pending(&mut self) -> Result<(), FrameError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let limits = self.available_limits()?;
        let batch = self
            .frame_decoder
            .as_mut()
            .ok_or(FrameError::QueueAccountingOverflow)?
            .push_bounded(&self.pending, limits)?;
        self.pending.drain(..batch.consumed);
        self.enqueue_frames(batch.frames)
    }

    fn available_limits(&self) -> Result<DecodeLimits, FrameError> {
        let records = MAX_READY_RECORDS
            .checked_sub(self.ready.len())
            .ok_or(FrameError::QueueAccountingOverflow)?;
        let payload_bytes = MAX_READY_BYTES
            .checked_sub(self.ready_bytes)
            .ok_or(FrameError::QueueAccountingOverflow)?;
        Ok(DecodeLimits {
            records,
            payload_bytes,
        })
    }

    fn enqueue_frames(&mut self, frames: Vec<Frame>) -> Result<(), FrameError> {
        for frame in frames {
            let next_records = self
                .ready
                .len()
                .checked_add(1)
                .ok_or(FrameError::QueueAccountingOverflow)?;
            let next_bytes = self
                .ready_bytes
                .checked_add(frame.payload.len())
                .ok_or(FrameError::QueueAccountingOverflow)?;
            if next_records > MAX_READY_RECORDS || next_bytes > MAX_READY_BYTES {
                return Err(FrameError::QueueAccountingOverflow);
            }
            self.ready.push_back(WireMessage::Frame(frame));
            self.ready_bytes = next_bytes;
        }
        Ok(())
    }

    fn pop_ready(&mut self) -> Result<Option<WireMessage>, FrameError> {
        let Some(message) = self.ready.pop_front() else {
            return Ok(None);
        };
        let bytes = match &message {
            WireMessage::ControlLine(line) => line.len(),
            WireMessage::Frame(frame) => frame.payload.len(),
        };
        self.ready_bytes = self
            .ready_bytes
            .checked_sub(bytes)
            .ok_or(FrameError::QueueAccountingOverflow)?;
        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests;
