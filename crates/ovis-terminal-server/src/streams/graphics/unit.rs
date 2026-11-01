//! One graphics unit: a single sequence, or a Kitty chunk series, kept or dropped whole.

use ovis_terminal_protocol::domain::graphics::{GraphicsCaps, GraphicsProtocol, SequenceAdmission};
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset};

use super::held::{Gated, Held, width};
use super::introducer::KittyHeader;

#[derive(Debug)]
pub(super) struct Unit {
    protocol: GraphicsProtocol,
    /// Held bytes; empty once the unit is dropped.
    held: Held,
    /// Raw offset of the unit's first byte.
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
        protocol: GraphicsProtocol,
        prefix: Held,
        (caps, out): (GraphicsCaps, &mut Gated),
    ) -> Self {
        let output_offset = prefix.raw_start;
        let mut unit = Self {
            protocol,
            held: Held::at(output_offset),
            output_offset,
            size: 0,
            dropped: false,
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

    /// Releases a unit that was not dropped, whole.
    pub(super) fn finish(self, out: &mut Gated) {
        if !self.dropped {
            out.graphics = out.graphics.saturating_add(width(self.held.bytes.len()));
            out.release(self.held);
        }
    }
}
