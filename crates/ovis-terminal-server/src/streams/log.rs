//! One terminal's filtered output as shared immutable chunks.
//!
//! Every viewer reads the same `Arc` chunks by [`LogOffset`]; a viewer's backlog is a
//! reference range, never a copy. Chunks are trimmed once no live viewer still needs
//! them, and each chunk keeps the removed-query positions that map log offsets back to
//! raw output offsets.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

use ovis_terminal_vt::OutputOffset;

use super::filter::Filtered;
use super::offset::LogOffset;

#[derive(Debug)]
struct Chunk {
    log: LogOffset,
    raw: OutputOffset,
    bytes: Arc<[u8]>,
    removed: Vec<(usize, u64)>,
}

impl Chunk {
    fn end(&self) -> LogOffset {
        self.log.advanced(self.bytes.len())
    }

    fn raw_at(&self, index: usize) -> OutputOffset {
        let removed: u64 = self
            .removed
            .iter()
            .filter(|(at, _)| *at <= index)
            .map(|(_, len)| *len)
            .sum();
        let index = u64::try_from(index).unwrap_or(u64::MAX);
        OutputOffset::new(self.raw.get().saturating_add(index).saturating_add(removed))
    }
}

/// A range of one shared chunk.
#[derive(Debug, Clone)]
pub(super) struct Shared {
    bytes: Arc<[u8]>,
    range: Range<usize>,
}

impl Shared {
    pub(super) const fn new(bytes: Arc<[u8]>, range: Range<usize>) -> Self {
        Self { bytes, range }
    }

    pub(super) fn as_slice(&self) -> &[u8] {
        self.bytes.get(self.range.clone()).unwrap_or_default()
    }

    pub(super) const fn len(&self) -> usize {
        self.range.end.saturating_sub(self.range.start)
    }

    /// Splits off the first `len` bytes, leaving the rest in `self`.
    pub(super) fn split_front(&mut self, len: usize) -> Self {
        let cut = self.range.start.saturating_add(len).min(self.range.end);
        let front = Self::new(Arc::clone(&self.bytes), self.range.start..cut);
        self.range.start = cut;
        front
    }
}

#[derive(Debug, Default)]
pub(super) struct OutputLog {
    chunks: VecDeque<Chunk>,
    end: LogOffset,
    raw_end: OutputOffset,
}

impl OutputLog {
    pub(super) fn append(&mut self, filtered: Filtered) {
        self.raw_end = filtered.raw_end;
        if filtered.bytes.is_empty() {
            return;
        }
        let chunk = Chunk {
            log: self.end,
            raw: filtered.raw_start,
            bytes: filtered.bytes.into(),
            removed: filtered.removed,
        };
        self.end = chunk.end();
        self.chunks.push_back(chunk);
    }

    pub(super) const fn end(&self) -> LogOffset {
        self.end
    }

    fn chunk_at(&self, at: LogOffset) -> Option<&Chunk> {
        let index = self.chunks.partition_point(|chunk| chunk.end() <= at);
        self.chunks.get(index).filter(|chunk| chunk.log <= at)
    }

    /// Up to `limit` bytes at `from`, or `None` when nothing is retained there.
    pub(super) fn read(&self, from: LogOffset, limit: usize) -> Option<Shared> {
        let chunk = self.chunk_at(from)?;
        let start = usize::try_from(from.since(chunk.log)).ok()?;
        let end = start.saturating_add(limit).min(chunk.bytes.len());
        (end > start).then(|| Shared::new(Arc::clone(&chunk.bytes), start..end))
    }

    /// The raw output offset of the byte that would be read at `at`.
    pub(super) fn raw_at(&self, at: LogOffset) -> OutputOffset {
        self.chunk_at(at).map_or(self.raw_end, |chunk| {
            chunk.raw_at(usize::try_from(at.since(chunk.log)).unwrap_or(usize::MAX))
        })
    }

    /// Drops chunks every live viewer has read past.
    pub(super) fn trim(&mut self, keep: LogOffset) {
        while self.chunks.front().is_some_and(|chunk| chunk.end() <= keep) {
            self.chunks.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_vt::OutputOffset;

    use super::super::filter::Filtered;
    use super::OutputLog;
    use crate::streams::offset::LogOffset;

    #[test]
    fn log_offsets_map_back_to_raw_offsets_across_removed_queries() {
        let mut log = OutputLog::default();
        log.append(Filtered {
            bytes: b"abcd".to_vec(),
            raw_start: OutputOffset::new(10),
            raw_end: OutputOffset::new(18),
            removed: vec![(2, 4)],
        });
        assert_eq!(log.raw_at(LogOffset::new(1)), OutputOffset::new(11));
        assert_eq!(log.raw_at(LogOffset::new(2)), OutputOffset::new(16));
        assert_eq!(log.raw_at(LogOffset::new(4)), OutputOffset::new(18));
        let read = log
            .read(LogOffset::new(1), 2)
            .map(|shared| shared.as_slice().to_vec());
        assert_eq!(read.as_deref(), Some(b"bc".as_slice()));
        log.trim(LogOffset::new(4));
        assert!(
            log.read(LogOffset::new(1), 2).is_none(),
            "trimmed chunks are released"
        );
    }
}
