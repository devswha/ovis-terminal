use super::{
    Channel, FRAME_HEADER_LEN, Frame, FrameError, FrameKind, MAX_FRAME_PAYLOAD, WireOffset,
};

#[derive(Debug)]
pub(super) struct DecodeBatch {
    pub(super) frames: Vec<Frame>,
    pub(super) consumed: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DecodeLimits {
    pub(super) records: usize,
    pub(super) payload_bytes: usize,
}

struct InputProgress<'a> {
    bytes: &'a [u8],
    consumed: usize,
}

impl InputProgress<'_> {
    const fn is_exhausted(&self) -> bool {
        self.consumed == self.bytes.len()
    }

    fn copy_to(&mut self, decoder: &mut FrameDecoder, target: usize) -> Result<(), FrameError> {
        let needed = target
            .checked_sub(decoder.pending.len())
            .ok_or(FrameError::QueueAccountingOverflow)?;
        let available = self
            .bytes
            .len()
            .checked_sub(self.consumed)
            .ok_or(FrameError::QueueAccountingOverflow)?;
        let count = needed.min(available);
        let end = self
            .consumed
            .checked_add(count)
            .ok_or(FrameError::QueueAccountingOverflow)?;
        let bytes = self
            .bytes
            .get(self.consumed..end)
            .ok_or(FrameError::QueueAccountingOverflow)?;
        let next_offset = decoder.byte_offset.checked_add(count)?;
        decoder.pending.extend_from_slice(bytes);
        decoder.byte_offset = next_offset;
        self.consumed = end;
        Ok(())
    }
}

/// Incrementally decodes bounded frames while retaining incomplete bytes.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    pending: Vec<u8>,
    expected_payload: Option<usize>,
    frame_offset: WireOffset,
    byte_offset: WireOffset,
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
        let batch = self.push_bounded(
            input,
            DecodeLimits {
                records: usize::MAX,
                payload_bytes: usize::MAX,
            },
        )?;
        if batch.consumed != input.len() {
            return Err(FrameError::QueueAccountingOverflow);
        }
        Ok(batch.frames)
    }

    pub(super) fn push_bounded(
        &mut self,
        input: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodeBatch, FrameError> {
        let mut frames = Vec::new();
        let mut output_bytes = 0_usize;
        let mut progress = InputProgress {
            bytes: input,
            consumed: 0,
        };
        loop {
            if self.pending.is_empty() {
                self.frame_offset = self.byte_offset;
            }
            if self.expected_payload.is_none() {
                if self.pending.len() == FRAME_HEADER_LEN {
                    self.expected_payload = Some(self.parse_header()?);
                    continue;
                }
                if frames.len() >= limits.records || progress.is_exhausted() {
                    break;
                }
                progress.copy_to(self, FRAME_HEADER_LEN)?;
                if self.pending.len() == FRAME_HEADER_LEN {
                    self.expected_payload = Some(self.parse_header()?);
                }
                continue;
            }

            let Some(payload_length) = self.expected_payload else {
                continue;
            };
            let next_record_count = frames
                .len()
                .checked_add(1)
                .ok_or(FrameError::QueueAccountingOverflow)?;
            let next_payload_bytes = output_bytes
                .checked_add(payload_length)
                .ok_or(FrameError::QueueAccountingOverflow)?;
            if next_record_count > limits.records || next_payload_bytes > limits.payload_bytes {
                break;
            }
            let frame_length =
                FRAME_HEADER_LEN
                    .checked_add(payload_length)
                    .ok_or(FrameError::OffsetOverflow {
                        offset: self.frame_offset,
                        amount: payload_length,
                    })?;
            if self.pending.len() == frame_length {
                frames.push(self.take_frame()?);
                output_bytes = next_payload_bytes;
                continue;
            }
            if progress.is_exhausted() {
                break;
            }
            progress.copy_to(self, frame_length)?;
            if self.pending.len() == frame_length {
                frames.push(self.take_frame()?);
                output_bytes = next_payload_bytes;
            }
        }
        Ok(DecodeBatch {
            frames,
            consumed: progress.consumed,
        })
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
    pub const fn byte_offset(&self) -> WireOffset {
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
