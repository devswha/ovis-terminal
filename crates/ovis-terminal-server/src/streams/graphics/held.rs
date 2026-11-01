//! Filtered bytes held back with the removed queries among them, and the gate's output.

use std::sync::Arc;

use ovis_terminal_core::terminals::images::ReleasedUnit;
use ovis_terminal_protocol::domain::graphics::{
    GraphicsDroppedData, GraphicsEvictedData, GraphicsProtocol,
};
use ovis_terminal_vt::OutputOffset;

use super::super::filter::Filtered;

pub(super) fn width(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Records `len` removed raw bytes standing before index `at`, merging with a removal
/// already there.
fn remove(list: &mut Vec<(usize, u64)>, index: usize, len: u64) {
    if len == 0 {
        return;
    }
    match list.last_mut() {
        Some((before, total)) if *before == index => *total = total.saturating_add(len),
        _ => list.push((index, len)),
    }
}

/// A contiguous raw span the gate has not released yet.
#[derive(Debug, Default)]
pub(super) struct Held {
    pub bytes: Vec<u8>,
    pub removed: Vec<(usize, u64)>,
    /// Raw offset the span begins at.
    pub raw_start: u64,
}

impl Held {
    pub(super) const fn at(raw_start: u64) -> Self {
        Self {
            bytes: Vec::new(),
            removed: Vec::new(),
            raw_start,
        }
    }

    pub(super) fn removal(&mut self, len: u64) {
        remove(&mut self.removed, self.bytes.len(), len);
    }

    /// Raw bytes the span accounts for.
    pub(super) fn raw_len(&self) -> u64 {
        self.removed
            .iter()
            .fold(width(self.bytes.len()), |sum, (_, len)| {
                sum.saturating_add(*len)
            })
    }

    pub(super) fn append(&mut self, other: Self) {
        let base = self.bytes.len();
        for (at, len) in other.removed {
            remove(&mut self.removed, base.saturating_add(at), len);
        }
        self.bytes.extend(other.bytes);
    }
}

/// Bytes the gate released, the graphics bytes among them and the notices of
/// sequences dropped in place.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::streams) struct Gated {
    pub filtered: Filtered,
    /// Bytes of admitted graphics sequences in `filtered.bytes`.
    pub graphics: u64,
    /// Dropped sequences: (index in `filtered.bytes` they stood before, notice).
    pub dropped: Vec<(usize, GraphicsDroppedData)>,
    /// Released graphics units: (index in `filtered.bytes` just past them, unit).
    pub units: Vec<(usize, ReleasedUnit)>,
    /// Store evictions: (index in `filtered.bytes` they stand before, notice).
    pub evicted: Vec<(usize, GraphicsEvictedData)>,
}

impl From<Filtered> for Gated {
    fn from(filtered: Filtered) -> Self {
        Self {
            filtered,
            ..Self::default()
        }
    }
}

impl Gated {
    pub(super) fn starting_at(raw_start: u64) -> Self {
        let mut gated = Self::default();
        gated.filtered.raw_start = OutputOffset::new(raw_start);
        gated
    }

    pub(super) fn push(&mut self, byte: u8) {
        self.filtered.bytes.push(byte);
    }

    /// Accounts `len` raw bytes that are not released here.
    pub(super) fn discard(&mut self, len: u64) {
        let at = self.filtered.bytes.len();
        remove(&mut self.filtered.removed, at, len);
    }

    pub(super) fn release(&mut self, held: Held) {
        let base = self.filtered.bytes.len();
        for (at, len) in held.removed {
            remove(&mut self.filtered.removed, base.saturating_add(at), len);
        }
        self.filtered.bytes.extend(held.bytes);
    }

    pub(super) fn notice(&mut self, notice: GraphicsDroppedData) {
        self.dropped.push((self.filtered.bytes.len(), notice));
    }

    /// Releases a whole graphics unit and reports it for the terminal's image store.
    pub(super) fn release_unit(&mut self, held: Held, unit: (u64, GraphicsProtocol)) {
        let bytes: Arc<[u8]> = Arc::from(held.bytes.as_slice());
        self.graphics = self.graphics.saturating_add(width(bytes.len()));
        self.release(held);
        let (key, protocol) = unit;
        let unit = ReleasedUnit::new(OutputOffset::new(key), protocol, bytes);
        self.units.push((self.filtered.bytes.len(), unit));
    }
}
