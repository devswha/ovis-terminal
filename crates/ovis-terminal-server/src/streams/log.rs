//! One terminal's filtered output as shared immutable chunks.
//!
//! Every viewer reads the same `Arc` chunks by [`LogOffset`]; a viewer's backlog is a
//! reference range, never a copy. Chunks are trimmed once no live viewer still needs
//! them, and each chunk keeps the removed-query positions that map log offsets back to
//! raw output offsets.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

use ovis_terminal_protocol::domain::graphics::{GraphicsDroppedData, GraphicsEvictedData};
use ovis_terminal_vt::OutputOffset;

use super::graphics::Gated;
use super::offset::LogOffset;

#[derive(Debug)]
struct Chunk {
    log: LogOffset,
    raw: OutputOffset,
    bytes: Arc<[u8]>,
    removed: Vec<(usize, u64)>,
    /// Bytes of whole graphics sequences in `bytes`.
    graphics: u64,
}

/// What a graphics notice reports.
#[derive(Debug)]
pub(super) enum NoticeData {
    /// A sequence over its cap was dropped whole here.
    Dropped(GraphicsDroppedData),
    /// Retaining the sequence before it evicted the oldest retained ones.
    Evicted(GraphicsEvictedData),
}

/// A graphics notice that stands where the log stood at `at`.
#[derive(Debug)]
pub(super) struct Notice {
    pub at: LogOffset,
    /// Increases by one per notice; a viewer remembers the last it was sent.
    pub seq: u64,
    pub data: NoticeData,
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
    notices: VecDeque<Notice>,
    notice_seq: u64,
}

impl OutputLog {
    pub(super) fn append(&mut self, gated: Gated) {
        let Gated {
            filtered,
            graphics,
            dropped,
            evicted,
            ..
        } = gated;
        let mut notices: Vec<(usize, NoticeData)> = dropped
            .into_iter()
            .map(|(index, data)| (index, NoticeData::Dropped(data)))
            .chain(
                evicted
                    .into_iter()
                    .map(|(index, data)| (index, NoticeData::Evicted(data))),
            )
            .collect();
        notices.sort_by_key(|(index, _)| *index);
        for (index, data) in notices {
            self.notice_seq = self.notice_seq.saturating_add(1);
            self.notices.push_back(Notice {
                at: self.end.advanced(index),
                seq: self.notice_seq,
                data,
            });
        }
        self.raw_end = filtered.raw_end;
        if filtered.bytes.is_empty() {
            return;
        }
        let chunk = Chunk {
            log: self.end,
            raw: filtered.raw_start,
            bytes: filtered.bytes.into(),
            removed: filtered.removed,
            graphics,
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

    /// Drops chunks and notices every live viewer has read past. With no viewer
    /// attached, a notice at `keep` itself goes too: a viewer attaching later skips it.
    pub(super) fn trim(&mut self, keep: LogOffset, unwatched: bool) {
        while self.chunks.front().is_some_and(|chunk| chunk.end() <= keep) {
            self.chunks.pop_front();
        }
        while self
            .notices
            .front()
            .is_some_and(|notice| notice.at < keep || (unwatched && notice.at <= keep))
        {
            self.notices.pop_front();
        }
    }

    /// Graphics notices still retained.
    #[cfg(test)]
    pub(super) fn notices(&self) -> usize {
        self.notices.len()
    }

    /// Graphics bytes in the chunks from `from` on.
    pub(super) fn graphics_after(&self, from: LogOffset) -> u64 {
        self.chunks
            .iter()
            .filter(|chunk| chunk.end() > from)
            .fold(0, |sum, chunk| sum.saturating_add(chunk.graphics))
    }

    /// The sequence number of the newest notice; a viewer starting now skips them all.
    pub(super) const fn notice_seq(&self) -> u64 {
        self.notice_seq
    }

    /// The first notice newer than `seen`.
    pub(super) fn notice_after(&self, seen: u64) -> Option<&Notice> {
        self.notices.iter().find(|notice| notice.seq > seen)
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::domain::graphics::{GraphicsCaps, GraphicsEvictedData};
    use ovis_terminal_protocol::schema::limits::Limits;
    use ovis_terminal_protocol::schema::scalar::{ByteCount, Count};
    use ovis_terminal_vt::OutputOffset;

    use super::super::filter::Filtered;
    use super::super::graphics::Gated;
    use super::OutputLog;
    use crate::streams::offset::LogOffset;

    #[test]
    fn log_offsets_map_back_to_raw_offsets_across_removed_queries() {
        let mut log = OutputLog::default();
        log.append(
            Filtered {
                bytes: b"abcd".to_vec(),
                raw_start: OutputOffset::new(10),
                raw_end: OutputOffset::new(18),
                removed: vec![(2, 4)],
            }
            .into(),
        );
        assert_eq!(log.raw_at(LogOffset::new(1)), OutputOffset::new(11));
        assert_eq!(log.raw_at(LogOffset::new(2)), OutputOffset::new(16));
        assert_eq!(log.raw_at(LogOffset::new(4)), OutputOffset::new(18));
        let read = log
            .read(LogOffset::new(1), 2)
            .map(|shared| shared.as_slice().to_vec());
        assert_eq!(read.as_deref(), Some(b"bc".as_slice()));
        log.trim(LogOffset::new(4), false);
        assert!(
            log.read(LogOffset::new(1), 2).is_none(),
            "trimmed chunks are released"
        );
    }

    #[test]
    fn a_notice_at_the_end_is_trimmed_only_when_no_viewer_is_attached() {
        let mut log = OutputLog::default();
        let mut gated = Gated::from(Filtered {
            bytes: b"ab".to_vec(),
            raw_start: OutputOffset::new(0),
            raw_end: OutputOffset::new(2),
            removed: Vec::new(),
        });
        let caps = GraphicsCaps::advertised(&Limits::v1());
        let evicted = GraphicsEvictedData::store_full(
            caps,
            (Count::new(1), ByteCount::new(1)),
            ByteCount::new(0),
        );
        gated.evicted.push((2, evicted));
        log.append(gated);
        log.trim(log.end(), false);
        assert_eq!(
            log.notices(),
            1,
            "a viewer at the end has not been sent it yet"
        );
        log.trim(log.end(), true);
        assert_eq!(log.notices(), 0);
    }
}
