//! One terminal's bounded store of the graphics sequences it still shows (plan todo 84).
//!
//! The attach stream releases every graphics sequence whole; the store keeps the bytes
//! of each one the VT engine still retains, keyed by the raw output offset where it
//! began, so a reattach can re-send them byte-identical. Its own entries are sized by
//! their bytes against `graphics_terminal_store_max_bytes` and the oldest is evicted
//! first; nothing is decoded, and nothing is written to the operations ledger. An image
//! the engine releases (scrolled out, erased, deleted) leaves the store at the next sync,
//! and the most recent releases are remembered for the replay record.

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;

use ovis_terminal_protocol::domain::graphics::{
    GraphicsCaps, GraphicsEvictedData, GraphicsProtocol, GraphicsReleaseReason, GraphicsReleased,
};
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, Count};
use ovis_terminal_vt::{ImageRelease, ImageSet, OutputOffset};

/// Releases remembered for the next replay record.
pub const RELEASED_LIMIT: usize = 64;

/// One graphics unit an attach stream released whole: a sequence, or one run of a Kitty
/// chunk series that other output interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReleasedUnit {
    /// Raw output offset where the sequence, or its series, began.
    pub key: OutputOffset,
    /// Its protocol.
    pub protocol: GraphicsProtocol,
    /// Its bytes, exactly as viewers received them.
    pub bytes: Arc<[u8]>,
}

impl ReleasedUnit {
    /// `bytes` of a `protocol` sequence that began at `key`.
    #[must_use]
    pub const fn new(key: OutputOffset, protocol: GraphicsProtocol, bytes: Arc<[u8]>) -> Self {
        Self {
            key,
            protocol,
            bytes,
        }
    }
}

#[derive(Debug)]
struct Stored {
    key: OutputOffset,
    pieces: Vec<Arc<[u8]>>,
    bytes: u64,
}

fn width(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

const fn reason(release: Option<ImageRelease>) -> GraphicsReleaseReason {
    match release {
        Some(ImageRelease::ScrolledOut) => GraphicsReleaseReason::ScrolledOut,
        Some(ImageRelease::Erased) => GraphicsReleaseReason::Erased,
        Some(ImageRelease::Deleted) => GraphicsReleaseReason::Deleted,
        Some(ImageRelease::Replaced) => GraphicsReleaseReason::Replaced,
        Some(ImageRelease::Reset) => GraphicsReleaseReason::Reset,
        Some(ImageRelease::Limit) => GraphicsReleaseReason::Limit,
        Some(_) | None => GraphicsReleaseReason::Expired,
    }
}

/// The graphics bytes one terminal can re-send, bounded by its store cap.
#[derive(Debug)]
pub struct ImageStore {
    caps: GraphicsCaps,
    entries: VecDeque<Stored>,
    bytes: u64,
    released: VecDeque<GraphicsReleased>,
}

impl ImageStore {
    /// An empty store bounded by `caps.terminal_store_max_bytes`.
    #[must_use]
    pub const fn new(caps: GraphicsCaps) -> Self {
        Self {
            caps,
            entries: VecDeque::new(),
            bytes: 0,
            released: VecDeque::new(),
        }
    }

    /// Keeps `unit` when `shown` retains its image, appending a later run of a series to
    /// its first; then evicts the oldest entries while the store is over its cap.
    /// Returns the eviction notice, if anything was evicted.
    pub fn admit(&mut self, unit: ReleasedUnit, shown: &ImageSet) -> Option<GraphicsEvictedData> {
        if !shown.retains(unit.key) {
            return None;
        }
        let size = width(&unit.bytes);
        match self.entries.iter_mut().find(|entry| entry.key == unit.key) {
            Some(entry) => {
                entry.pieces.push(unit.bytes);
                entry.bytes = entry.bytes.saturating_add(size);
            }
            None => self.entries.push_back(Stored {
                key: unit.key,
                pieces: vec![unit.bytes],
                bytes: size,
            }),
        }
        self.bytes = self.bytes.saturating_add(size);
        self.evict()
    }

    fn evict(&mut self) -> Option<GraphicsEvictedData> {
        let cap = self.caps.terminal_store_max_bytes.get();
        let (mut sequences, mut bytes) = (0_u64, 0_u64);
        while self.bytes > cap {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(oldest.bytes);
            sequences = sequences.saturating_add(1);
            bytes = bytes.saturating_add(oldest.bytes);
            self.note(oldest.key, GraphicsReleaseReason::StoreFull);
        }
        (sequences > 0).then(|| {
            GraphicsEvictedData::store_full(
                self.caps,
                (Count::new(sequences), ByteCount::new(bytes)),
                ByteCount::new(self.bytes),
            )
        })
    }

    /// Forgets every entry `shown` no longer retains, remembering the engine's reason.
    pub fn sync(&mut self, shown: &ImageSet) {
        let (kept, gone): (VecDeque<Stored>, VecDeque<Stored>) = self
            .entries
            .drain(..)
            .partition(|entry| shown.retains(entry.key));
        self.entries = kept;
        for entry in gone {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            self.note(entry.key, reason(shown.release_of(entry.key)));
        }
    }

    /// Forgets the series beginning at `key` that the stream dropped whole over the
    /// sequence cap; its drop notice already reported it.
    pub fn forget(&mut self, key: OutputOffset) {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key)
            && let Some(entry) = self.entries.remove(index)
        {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
        }
    }

    fn note(&mut self, key: OutputOffset, why: GraphicsReleaseReason) {
        self.released
            .push_back(GraphicsReleased::new(ByteOffset::new(key.get()), why));
        while self.released.len() > RELEASED_LIMIT {
            self.released.pop_front();
        }
    }

    /// The bytes of the sequence that began at `key`, in the runs it arrived in.
    pub fn pieces(&self, key: OutputOffset) -> Option<&[Arc<[u8]>]> {
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.pieces.as_slice())
    }

    /// The most recently released sequences, oldest first.
    pub fn released(&self) -> Vec<GraphicsReleased> {
        self.released.iter().cloned().collect()
    }

    /// Bytes retained now; never above the store cap after [`ImageStore::admit`].
    pub const fn retained_bytes(&self) -> u64 {
        self.bytes
    }
}
