//! One graphics unit: a single sequence, or a Kitty chunk series, kept or dropped whole.

use ovis_terminal_protocol::domain::graphics::{GraphicsCaps, GraphicsProtocol, SequenceAdmission};
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset};

use super::held::{Gated, Held, width};
use super::introducer::KittyHeader;

/// A Kitty transmission other output interrupted between two chunks: its later chunks
/// continue it, measured against the cap together with what it already released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Series {
    start: u64,
    size: u64,
    dropped: bool,
}

/// What a new unit begins: its protocol and the interrupted series it continues.
#[derive(Debug, Clone, Copy)]
pub(super) struct Start {
    pub protocol: GraphicsProtocol,
    pub series: Option<Series>,
}

#[derive(Debug)]
pub(super) struct Unit {
    protocol: GraphicsProtocol,
    /// Held bytes; empty once the unit is dropped.
    held: Held,
    /// Raw offset of the first byte of the unit, or of the series it continues.
    output_offset: u64,
    /// Sequence bytes seen, counted against the cap.
    size: u64,
    dropped: bool,
    /// Inside a sequence, rather than between two chunks of a Kitty series.
    pub open: bool,
    header: KittyHeader,
}

impl Unit {
    /// A unit whose introducer is `prefix`.
    pub(super) fn begin(
        start: Start,
        prefix: Held,
        (caps, out): (GraphicsCaps, &mut Gated),
    ) -> Self {
        let Start { protocol, series } = start;
        let mut unit = Self {
            protocol,
            held: Held::at(prefix.raw_start),
            output_offset: series.map_or(prefix.raw_start, |series| series.start),
            size: series.map_or(0, |series| series.size),
            dropped: series.is_some_and(|series| series.dropped),
            open: true,
            header: KittyHeader::default(),
        };
        unit.keep(prefix, (caps, out));
        unit
    }

    pub(super) const fn protocol(&self) -> GraphicsProtocol {
        self.protocol
    }

    /// Where its held bytes begin, unless it was dropped.
    pub(super) const fn held_start(&self) -> Option<u64> {
        if self.dropped {
            None
        } else {
            Some(self.held.raw_start)
        }
    }

    /// Whether a Kitty chunk that just ended announced another.
    pub(super) fn awaits_chunk(&self) -> bool {
        self.protocol == GraphicsProtocol::Kitty && self.header.more()
    }

    /// The next chunk of the series begins with `prefix`.
    pub(super) fn next_chunk(&mut self, prefix: Held, (caps, out): (GraphicsCaps, &mut Gated)) {
        self.open = true;
        self.header = KittyHeader::default();
        self.keep(prefix, (caps, out));
    }

    pub(super) fn take(&mut self, byte: u8, (caps, out): (GraphicsCaps, &mut Gated)) {
        self.header.feed(byte);
        self.size = self.size.saturating_add(1);
        if self.dropped {
            out.discard(1);
        } else {
            self.held.bytes.push(byte);
            self.check(caps, out);
        }
    }

    /// Takes held bytes that belong to this unit, such as a terminator.
    pub(super) fn absorb(&mut self, held: Held, (caps, out): (GraphicsCaps, &mut Gated)) {
        for byte in &held.bytes {
            self.header.feed(*byte);
        }
        self.keep(held, (caps, out));
    }

    fn keep(&mut self, held: Held, (caps, out): (GraphicsCaps, &mut Gated)) {
        self.size = self.size.saturating_add(width(held.bytes.len()));
        if self.dropped {
            out.discard(held.raw_len());
        } else {
            self.held.append(held);
            self.check(caps, out);
        }
    }

    pub(super) fn removal(&mut self, len: u64, out: &mut Gated) {
        if self.dropped {
            out.discard(len);
        } else {
            self.held.removal(len);
        }
    }

    /// Drops the unit the moment it passes the cap: its bytes leave unreleased and one
    /// notice stands where it began.
    fn check(&mut self, caps: GraphicsCaps, out: &mut Gated) {
        let measured = (
            ByteOffset::new(self.output_offset),
            ByteCount::new(self.size),
        );
        if let SequenceAdmission::Dropped(notice) = caps.admit(self.protocol, measured) {
            out.notice(notice);
            out.discard(std::mem::take(&mut self.held).raw_len());
            self.dropped = true;
        }
    }

    /// The series this unit leaves open: a Kitty chunk announced another.
    pub(super) fn carried(&self) -> Option<Series> {
        self.awaits_chunk().then_some(Series {
            start: self.output_offset,
            size: self.size,
            dropped: self.dropped,
        })
    }

    /// Releases a unit that was not dropped, whole.
    pub(super) fn finish(self, out: &mut Gated) {
        if !self.dropped {
            out.release_unit(self.held, (self.output_offset, self.protocol));
        }
    }
}
