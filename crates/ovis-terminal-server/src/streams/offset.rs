//! The byte-offset domains of an attach stream.
//!
//! Raw holder output is counted by [`ovis_terminal_vt::OutputOffset`]. The terminal's
//! filtered log drops VT-answered queries and is counted by [`LogOffset`]. Each viewer's
//! delivery adds its own checkpoint paints and is counted by [`StreamOffset`]; CREDIT
//! acknowledges only that last domain, so removed bytes can never fabricate credit.

use ovis_terminal_protocol::schema::scalar::ByteOffset;

fn width(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Offset in a terminal's filtered output log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LogOffset(u64);

impl LogOffset {
    /// The start of the log.
    pub const ZERO: Self = Self(0);

    /// Wraps a filtered byte count.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The filtered byte count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(super) fn advanced(self, len: usize) -> Self {
        Self(self.0.saturating_add(width(len)))
    }

    /// Bytes from `earlier` up to this offset; zero when `earlier` is not earlier.
    pub(super) const fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// Offset in one viewer's delivery: checkpoint paints plus filtered live bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StreamOffset(u64);

impl StreamOffset {
    /// The first byte of a stream.
    pub const ZERO: Self = Self(0);

    /// Wraps a delivered byte count.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The delivered byte count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The offset as carried on the wire, a canonical decimal string.
    #[must_use]
    pub const fn wire(self) -> ByteOffset {
        ByteOffset::new(self.0)
    }

    pub(super) fn advanced(self, len: usize) -> Self {
        Self(self.0.saturating_add(width(len)))
    }

    pub(super) const fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}
