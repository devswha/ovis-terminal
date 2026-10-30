use super::{Channel, FRAME_HEADER_LEN, Frame, FrameError, FrameKind, MAX_FRAME_PAYLOAD};

/// Incrementally decodes bounded frames while retaining incomplete bytes.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    pending: Vec<u8>,
    expected_payload: Option<usize>,
    frame_offset: u64,
    byte_offset: u64,
}

impl FrameDecoder {
    /// Creates a decoder at byte offset zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes and returns each complete frame, retaining any partial frame.
    ///
    /// # Errors
    /// Returns a typed error for an oversized frame, unknown kind, or offset overflow.
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<Frame>, FrameError> {
        let mut frames = Vec::new();
        let mut consumed = 0_usize;
        loop {
            if self.pending.is_empty() {
                self.frame_offset = self.byte_offset;
            }
            let target = match self.expected_payload {
                Some(payload) => {
                    FRAME_HEADER_LEN
                        .checked_add(payload)
                        .ok_or(FrameError::OffsetOverflow {
                            offset: self.frame_offset,
                            amount: payload,
                        })?
                }
                None => FRAME_HEADER_LEN,
            };
            if self.pending.len() < target && consumed < input.len() {
                let needed =
                    target
                        .checked_sub(self.pending.len())
                        .ok_or(FrameError::OffsetOverflow {
                            offset: self.frame_offset,
                            amount: target,
                        })?;
                let available =
                    input
                        .len()
                        .checked_sub(consumed)
                        .ok_or(FrameError::OffsetOverflow {
                            offset: self.byte_offset,
                            amount: input.len(),
                        })?;
                let count = needed.min(available);
                let end = consumed
                    .checked_add(count)
                    .ok_or(FrameError::OffsetOverflow {
                        offset: self.byte_offset,
                        amount: count,
                    })?;
                let bytes = input.get(consumed..end).ok_or(FrameError::OffsetOverflow {
                    offset: self.byte_offset,
                    amount: count,
                })?;
                let next_offset = advance_offset(self.byte_offset, count)?;
                self.pending.extend_from_slice(bytes);
                self.byte_offset = next_offset;
                consumed = end;
            }
            if self.pending.len() == FRAME_HEADER_LEN && self.expected_payload.is_none() {
                self.expected_payload = Some(self.parse_header()?);
            }
            if let Some(payload_length) = self.expected_payload {
                let frame_length = FRAME_HEADER_LEN.checked_add(payload_length).ok_or(
                    FrameError::OffsetOverflow {
                        offset: self.frame_offset,
                        amount: payload_length,
                    },
                )?;
                if self.pending.len() == frame_length {
                    frames.push(self.take_frame()?);
                    continue;
                }
            }
            if consumed == input.len() {
                break;
            }
        }
        Ok(frames)
    }

    /// Reports an incomplete header or payload at end-of-stream.
    ///
    /// # Errors
    /// Returns [`FrameError::TruncatedHeader`] or [`FrameError::TruncatedPayload`].
    pub fn finish(&self) -> Result<(), FrameError> {
        match self.expected_payload {
            None if self.pending.is_empty() => Ok(()),
            None => Err(FrameError::TruncatedHeader {
                offset: self.frame_offset,
                actual: self.pending.len(),
            }),
            Some(expected) => {
                let actual = self.pending.len().checked_sub(FRAME_HEADER_LEN).ok_or(
                    FrameError::TruncatedHeader {
                        offset: self.frame_offset,
                        actual: self.pending.len(),
                    },
                )?;
                Err(FrameError::TruncatedPayload {
                    offset: self.frame_offset,
                    expected,
                    actual,
                })
            }
        }
    }

    /// Returns the absolute offset of the next byte accepted by this decoder.
    #[must_use]
    pub const fn byte_offset(&self) -> u64 {
        self.byte_offset
    }

    /// Returns the number of retained header and payload bytes.
    #[must_use]
    pub const fn buffered_len(&self) -> usize {
        self.pending.len()
    }

    fn parse_header(&self) -> Result<usize, FrameError> {
        let raw_kind = self
            .pending
            .get(4)
            .copied()
            .ok_or(FrameError::TruncatedHeader {
                offset: self.frame_offset,
                actual: self.pending.len(),
            })?;
        let length_bytes: [u8; 4] = self
            .pending
            .get(5..FRAME_HEADER_LEN)
            .ok_or(FrameError::TruncatedHeader {
                offset: self.frame_offset,
                actual: self.pending.len(),
            })?
            .try_into()
            .map_err(|_| FrameError::TruncatedHeader {
                offset: self.frame_offset,
                actual: self.pending.len(),
            })?;
        FrameKind::try_from(raw_kind).map_err(|kind| FrameError::UnknownKind {
            kind,
            offset: self.frame_offset,
        })?;
        let length = usize::try_from(u32::from_le_bytes(length_bytes)).map_err(|_| {
            FrameError::FrameTooLarge {
                length: usize::MAX,
                limit: MAX_FRAME_PAYLOAD,
                offset: self.frame_offset,
            }
        })?;
        if length > MAX_FRAME_PAYLOAD {
            return Err(FrameError::FrameTooLarge {
                length,
                limit: MAX_FRAME_PAYLOAD,
                offset: self.frame_offset,
            });
        }
        Ok(length)
    }

    fn take_frame(&mut self) -> Result<Frame, FrameError> {
        let channel = Channel::new(u32::from_le_bytes(
            self.pending
                .get(0..4)
                .ok_or(FrameError::TruncatedHeader {
                    offset: self.frame_offset,
                    actual: self.pending.len(),
                })?
                .try_into()
                .map_err(|_| FrameError::TruncatedHeader {
                    offset: self.frame_offset,
                    actual: self.pending.len(),
                })?,
        ));
        let raw_kind = self
            .pending
            .get(4)
            .copied()
            .ok_or(FrameError::TruncatedHeader {
                offset: self.frame_offset,
                actual: self.pending.len(),
            })?;
        let kind = FrameKind::try_from(raw_kind).map_err(|kind| FrameError::UnknownKind {
            kind,
            offset: self.frame_offset,
        })?;
        let payload = self
            .pending
            .get(FRAME_HEADER_LEN..)
            .ok_or(FrameError::TruncatedHeader {
                offset: self.frame_offset,
                actual: self.pending.len(),
            })?
            .to_vec();
        let frame = Frame::new(channel, kind, payload)?;
        self.pending.clear();
        self.expected_payload = None;
        Ok(frame)
    }
}

fn advance_offset(offset: u64, amount: usize) -> Result<u64, FrameError> {
    let amount_u64 =
        u64::try_from(amount).map_err(|_| FrameError::OffsetOverflow { offset, amount })?;
    offset
        .checked_add(amount_u64)
        .ok_or(FrameError::OffsetOverflow { offset, amount })
}
