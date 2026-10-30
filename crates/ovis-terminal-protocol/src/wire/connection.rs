use std::collections::VecDeque;

use super::{Frame, FrameDecoder, FrameError, MAX_FRAME_PAYLOAD};

/// A complete message in either persistent NDJSON or upgraded framed mode.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireMessage {
    /// One UTF-8 NDJSON record without its terminating newline.
    ControlLine(Vec<u8>),
    /// One typed channel frame.
    Frame(Frame),
}

/// Persistent connection decoder with an explicit hello upgrade boundary.
#[derive(Debug, Default)]
pub struct ConnectionDecoder {
    pending: Vec<u8>,
    ready: VecDeque<WireMessage>,
    frame_decoder: Option<FrameDecoder>,
    plain_offset: u64,
}

impl ConnectionDecoder {
    /// Creates a decoder in persistent NDJSON control mode.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffers one transport read without treating read boundaries as records.
    ///
    /// # Errors
    /// Returns an error if pending control bytes exceed one bounded upgrade read.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        if let Some(decoder) = self.frame_decoder.as_mut() {
            self.ready
                .extend(decoder.push(bytes)?.into_iter().map(WireMessage::Frame));
            return Ok(());
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
        Ok(())
    }

    /// Returns the next complete line or frame, if one is available.
    ///
    /// # Errors
    /// Returns a typed error for invalid UTF-8, an overlong line, or malformed frame bytes.
    pub fn next_message(&mut self) -> Result<Option<WireMessage>, FrameError> {
        if let Some(message) = self.ready.pop_front() {
            return Ok(Some(message));
        }
        if let Some(decoder) = self.frame_decoder.as_mut() {
            let frames = decoder.push(&[])?;
            self.ready
                .extend(frames.into_iter().map(WireMessage::Frame));
            return Ok(self.ready.pop_front());
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
            .ok_or(FrameError::OffsetOverflow {
                offset: self.plain_offset,
                amount: line_end,
            })?;
        std::str::from_utf8(line_bytes).map_err(|_| FrameError::InvalidControlUtf8 {
            offset: self.plain_offset,
        })?;
        let next_offset = advance_plain_offset(self.plain_offset, line_length)?;
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
        let mut decoder = FrameDecoder::new();
        let frames = decoder.push(&self.pending)?;
        self.pending.clear();
        self.ready
            .extend(frames.into_iter().map(WireMessage::Frame));
        self.frame_decoder = Some(decoder);
        Ok(())
    }
}

fn advance_plain_offset(offset: u64, amount: usize) -> Result<u64, FrameError> {
    let amount_u64 =
        u64::try_from(amount).map_err(|_| FrameError::OffsetOverflow { offset, amount })?;
    offset
        .checked_add(amount_u64)
        .ok_or(FrameError::OffsetOverflow { offset, amount })
}
