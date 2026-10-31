//! Bounded raw output retention; eviction advances a visible floor.

use std::collections::VecDeque;

use ovis_terminal_vt::OutputOffset;

use super::error::ScreenError;
use super::requests::OutputRequest;
use super::views::{CheckpointMark, OutputSlice, RetainedRange};

fn widen(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

#[derive(Debug)]
pub(super) struct OutputRing {
    bytes: VecDeque<u8>,
    floor: OutputOffset,
    capacity: usize,
}

impl OutputRing {
    pub(super) const fn new(capacity: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            floor: OutputOffset::ZERO,
            capacity,
        }
    }

    pub(super) fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
        let excess = self.bytes.len().saturating_sub(self.capacity);
        self.bytes.drain(..excess);
        self.floor = OutputOffset::new(self.floor.get().saturating_add(widen(excess)));
    }

    pub(super) fn range(&self) -> RetainedRange {
        RetainedRange {
            floor: self.floor,
            head: OutputOffset::new(self.floor.get().saturating_add(widen(self.bytes.len()))),
        }
    }

    /// Reads retained bytes; an evicted start names the checkpoint that still covers the range.
    pub(super) fn read(
        &self,
        request: &OutputRequest,
        checkpoint: Option<&CheckpointMark>,
    ) -> Result<OutputSlice, ScreenError> {
        let retained = self.range();
        let from = request.from();
        if from < retained.floor {
            return Err(ScreenError::HistoryGap {
                requested: from,
                retained,
                checkpoint: checkpoint
                    .filter(|mark| mark.offset >= retained.floor)
                    .cloned(),
            });
        }
        if from > retained.head {
            return Err(ScreenError::InvalidRange { field: "from" });
        }
        let skip = usize::try_from(from.get().saturating_sub(retained.floor.get()))
            .map_err(|_| ScreenError::InvalidRange { field: "from" })?;
        let limit = usize::try_from(request.limit_bytes()).unwrap_or(usize::MAX);
        let bytes: Vec<u8> = self.bytes.iter().skip(skip).take(limit).copied().collect();
        let next = OutputOffset::new(from.get().saturating_add(widen(bytes.len())));
        Ok(OutputSlice {
            from,
            next,
            truncated: next < retained.head,
            bytes,
            retained,
        })
    }
}
