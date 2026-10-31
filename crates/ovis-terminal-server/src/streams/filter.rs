//! Removes VT-answered terminal queries from downstream bytes.
//!
//! The core answers a query once on the holder's internal lane; a viewer that saw the
//! same query would answer it again. The filter tracks escape-sequence boundaries across
//! reads and drops a complete sequence exactly when the VT reported an answered query
//! ending at that raw offset. An incomplete sequence is held back until it completes, so
//! a query split across reads is removed whole. Raw output itself is retained unchanged
//! by the screen for recovery.

use std::mem;

use ovis_terminal_vt::OutputOffset;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
/// Longest incomplete sequence held back; longer ones are released unfiltered.
const HOLD_LIMIT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    Ground,
    Escape,
    Csi,
    Text,
    TextEscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Continue,
    Begin,
    End,
    Abort,
    /// A string ended at the previous ESC, which begins a new sequence.
    EndBeforeEscape,
}

const fn step(scan: Scan, byte: u8) -> (Scan, Step) {
    match (scan, byte) {
        (Scan::Ground | Scan::Escape | Scan::Csi, ESC) => (Scan::Escape, Step::Begin),
        (Scan::Ground, _) => (Scan::Ground, Step::Continue),
        (_, CAN | SUB) => (Scan::Ground, Step::Abort),
        (Scan::Escape, b'[') => (Scan::Csi, Step::Continue),
        (Scan::Escape, b']' | b'P' | b'X' | b'^' | b'_') => (Scan::Text, Step::Continue),
        (Scan::Escape, 0x30..=0x7e) | (Scan::Csi, 0x40..=0x7e) | (Scan::Text, BEL) => {
            (Scan::Ground, Step::End)
        }
        (Scan::Text, ESC) => (Scan::TextEscape, Step::Continue),
        (Scan::TextEscape, b'\\') => (Scan::Ground, Step::End),
        (Scan::TextEscape, _) => (Scan::Escape, Step::EndBeforeEscape),
        (state, _) => (state, Step::Continue),
    }
}

/// Raw bytes contiguous with everything the filter saw before.
#[derive(Debug, Clone, Copy)]
pub(super) struct RawSlice<'a> {
    /// Raw offset of `bytes[0]`.
    pub at: OutputOffset,
    /// The bytes.
    pub bytes: &'a [u8],
    /// End offsets of queries the VT answered.
    pub answered: &'a [OutputOffset],
}

/// Bytes the filter released and the raw span they account for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Filtered {
    /// Released bytes.
    pub bytes: Vec<u8>,
    /// Raw offset of the first byte this release accounts for.
    pub raw_start: OutputOffset,
    /// Raw offset after the last accounted byte; later raw bytes are still held.
    pub raw_end: OutputOffset,
    /// Removed queries: (index in `bytes` they stood before, raw length), in order.
    pub removed: Vec<(usize, u64)>,
}

#[derive(Debug, Default)]
pub(super) struct QueryFilter {
    held: Vec<u8>,
    held_at: OutputOffset,
}

struct Pass<'a> {
    buffer: &'a [u8],
    start: u64,
    answered: &'a [OutputOffset],
    out: Filtered,
    copied: usize,
}

impl Pass<'_> {
    fn complete(&mut self, begin: usize, end: usize) {
        let raw_end = OutputOffset::new(self.start.saturating_add(width(end)));
        if !self.answered.contains(&raw_end) {
            return;
        }
        self.copy_to(begin);
        let at = self.out.bytes.len();
        self.out
            .removed
            .push((at, width(end.saturating_sub(begin))));
        self.copied = end;
    }

    fn copy_to(&mut self, index: usize) {
        if let Some(bytes) = self.buffer.get(self.copied..index) {
            self.out.bytes.extend_from_slice(bytes);
        }
        self.copied = self.copied.max(index);
    }
}

fn width(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

impl QueryFilter {
    /// Filters `raw`, holding back a trailing incomplete escape sequence.
    pub(super) fn apply(&mut self, raw: RawSlice<'_>) -> Filtered {
        let start = if self.held.is_empty() {
            raw.at
        } else {
            self.held_at
        };
        let mut buffer = mem::take(&mut self.held);
        buffer.extend_from_slice(raw.bytes);
        let mut pass = Pass {
            buffer: &buffer,
            start: start.get(),
            answered: raw.answered,
            out: Filtered {
                raw_start: start,
                ..Filtered::default()
            },
            copied: 0,
        };
        let open = scan(&mut pass);
        let held_from = open.filter(|&begin| buffer.len().saturating_sub(begin) <= HOLD_LIMIT);
        let keep = held_from.unwrap_or(buffer.len());
        pass.copy_to(keep);
        let mut out = pass.out;
        out.raw_end = OutputOffset::new(start.get().saturating_add(width(keep)));
        if let Some(tail) = buffer.get(keep..) {
            self.held = tail.to_vec();
            self.held_at = out.raw_end;
        }
        out
    }

    /// Releases held bytes unfiltered, so the log reaches the screen's live offset.
    pub(super) fn flush(&mut self) -> Filtered {
        let bytes = mem::take(&mut self.held);
        let raw_start = self.held_at;
        let raw_end = OutputOffset::new(raw_start.get().saturating_add(width(bytes.len())));
        self.held_at = raw_end;
        Filtered {
            bytes,
            raw_start,
            raw_end,
            removed: Vec::new(),
        }
    }
}

/// Walks the buffer, removing answered sequences; returns where an open sequence began.
fn scan(pass: &mut Pass<'_>) -> Option<usize> {
    let mut state = Scan::Ground;
    let mut begin = None;
    let mut index = 0;
    while let Some(&byte) = pass.buffer.get(index) {
        let (next, event) = step(state, byte);
        state = next;
        match event {
            Step::Continue => {}
            Step::Begin => begin = Some(index),
            Step::Abort => begin = None,
            Step::End => {
                if let Some(first) = begin.take() {
                    pass.complete(first, index.saturating_add(1));
                }
            }
            Step::EndBeforeEscape => {
                let escape = index.saturating_sub(1);
                if let Some(first) = begin.replace(escape) {
                    pass.complete(first, escape);
                }
                // The byte after the terminating ESC is read in the escape state.
                continue;
            }
        }
        index = index.saturating_add(1);
    }
    begin
}

#[cfg(test)]
mod tests;
