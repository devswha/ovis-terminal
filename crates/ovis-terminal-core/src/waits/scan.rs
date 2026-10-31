//! Incremental byte-pattern search over ordered output chunks.

use ovis_terminal_vt::OutputOffset;

fn widen(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Result of scanning one chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// The pattern starts at this offset.
    Found(OutputOffset),
    /// Not yet; scanning continues at the next chunk.
    Pending,
    /// The chunk starts after the scanned position; bytes between were never seen.
    Behind,
}

/// Search state: the next unscanned offset and the bytes that may begin a match.
#[derive(Debug, Clone)]
pub(super) struct Scan {
    needle: Vec<u8>,
    carry: Vec<u8>,
    position: OutputOffset,
}

impl Scan {
    pub(super) const fn new(needle: Vec<u8>, from: OutputOffset) -> Self {
        Self {
            needle,
            carry: Vec::new(),
            position: from,
        }
    }

    pub(super) const fn position(&self) -> OutputOffset {
        self.position
    }

    /// Where a later wait must start so a match spanning the carried bytes is not lost.
    pub(super) fn resume(&self) -> OutputOffset {
        OutputOffset::new(self.position.get().saturating_sub(widen(self.carry.len())))
    }

    pub(super) fn after(&self, at: OutputOffset) -> OutputOffset {
        OutputOffset::new(at.get().saturating_add(widen(self.needle.len())))
    }

    /// Scans `bytes` that start at `from`, skipping any prefix already scanned.
    pub(super) fn feed(&mut self, from: OutputOffset, bytes: &[u8]) -> Step {
        if from > self.position {
            return Step::Behind;
        }
        let seen =
            usize::try_from(self.position.get().saturating_sub(from.get())).unwrap_or(usize::MAX);
        let Some(fresh) = bytes.get(seen..) else {
            return Step::Pending;
        };
        let mut window = std::mem::take(&mut self.carry);
        window.extend_from_slice(fresh);
        let base = self
            .position
            .get()
            .saturating_sub(widen(window.len().saturating_sub(fresh.len())));
        if let Some(index) = window
            .windows(self.needle.len())
            .position(|candidate| candidate == self.needle.as_slice())
        {
            return Step::Found(OutputOffset::new(base.saturating_add(widen(index))));
        }
        self.position = OutputOffset::new(self.position.get().saturating_add(widen(fresh.len())));
        let keep = self.needle.len().saturating_sub(1).min(window.len());
        self.carry = window.split_off(window.len().saturating_sub(keep));
        Step::Pending
    }
}
