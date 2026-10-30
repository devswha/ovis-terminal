use super::{FrameError, MAX_FRAME_PAYLOAD};

/// Continuation and completeness fields for one part of a logical result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChunkMetadata {
    /// Total byte length of the complete logical result.
    pub total: u64,
    /// Absolute byte offset of this chunk in that result.
    pub offset: u64,
    /// Offset to request next, or `None` after the final chunk.
    pub continuation: Option<u64>,
    /// Whether bytes are absent from the logical result represented here.
    pub truncated: bool,
}

/// One borrowed, frame-bounded slice of a logical result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LogicalChunk<'a> {
    /// Bytes carried by this chunk.
    pub payload: &'a [u8],
    /// Machine-readable range and continuation information.
    pub metadata: ChunkMetadata,
}

/// A complete result that can be returned in bounded frame-sized chunks.
#[derive(Debug, Clone, Copy)]
pub struct LogicalResult<'a> {
    bytes: &'a [u8],
    total: u64,
}

impl<'a> LogicalResult<'a> {
    /// Creates a complete logical result from its bytes.
    ///
    /// # Errors
    /// Returns [`FrameError::OffsetOverflow`] if its length cannot fit a wire offset.
    pub fn new(bytes: &'a [u8]) -> Result<Self, FrameError> {
        let total = u64::try_from(bytes.len()).map_err(|_| FrameError::OffsetOverflow {
            offset: 0,
            amount: bytes.len(),
        })?;
        Ok(Self { bytes, total })
    }

    /// Returns an iterator yielding payloads no larger than the frame ceiling.
    #[must_use]
    pub const fn chunks(self) -> LogicalChunks<'a> {
        LogicalChunks {
            bytes: self.bytes,
            total: self.total,
            cursor: 0,
        }
    }
}

/// Iterator over a logical result's bounded chunks.
#[derive(Debug, Clone)]
pub struct LogicalChunks<'a> {
    bytes: &'a [u8],
    total: u64,
    cursor: usize,
}

impl<'a> Iterator for LogicalChunks<'a> {
    type Item = Result<LogicalChunk<'a>, FrameError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.bytes.len() {
            return None;
        }
        let end = self
            .cursor
            .saturating_add(MAX_FRAME_PAYLOAD)
            .min(self.bytes.len());
        let Some(payload) = self.bytes.get(self.cursor..end) else {
            return Some(Err(FrameError::OffsetOverflow {
                offset: self.total,
                amount: self.cursor,
            }));
        };
        let Ok(offset) = u64::try_from(self.cursor) else {
            return Some(Err(FrameError::OffsetOverflow {
                offset: self.total,
                amount: self.cursor,
            }));
        };
        let continuation = if end < self.bytes.len() {
            let Ok(next) = u64::try_from(end) else {
                return Some(Err(FrameError::OffsetOverflow {
                    offset: self.total,
                    amount: end,
                }));
            };
            Some(next)
        } else {
            None
        };
        self.cursor = end;
        Some(Ok(LogicalChunk {
            payload,
            metadata: ChunkMetadata {
                total: self.total,
                offset,
                continuation,
                truncated: false,
            },
        }))
    }
}
