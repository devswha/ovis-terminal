//! Kitty, iTerm2 and sixel graphics through attach streams, whole or not at all (plan
//! todo 83).
//!
//! The gate reads the query filter's output with the filter's own escape-boundary
//! tracker and recognises a graphics sequence by its introducer alone: `APC _G` (with
//! every following `m=1` chunk of the same transmission), `OSC 1337;File=` and
//! `DCS ... q`. It holds such a unit until its last byte arrives, then releases it in
//! one piece, so no viewer is ever handed a prefix. A unit that passes
//! `graphics_sequence_max_bytes` is dropped whole the moment it does: its bytes are
//! accounted as removed raw output, never buffered further, and one notice stands where
//! it began. A Kitty series that other output interrupts stays open until a chunk ends
//! it, and its runs are measured against the cap together (plan todo 84). Payloads are
//! never decoded; the VT has already drawn every raw byte.

mod backlog;
mod held;
mod introducer;
mod replay;
#[cfg(test)]
mod tests;
mod unit;

use ovis_terminal_protocol::domain::graphics::{GraphicsCaps, GraphicsProtocol};
use ovis_terminal_vt::OutputOffset;

pub(super) use self::backlog::over_backlog;
pub(super) use self::held::Gated;
use self::held::Held;
use self::introducer::{Verdict, classify};
pub(super) use self::replay::{Painted, Piece, admit, notice_record, painted};
use self::unit::{Series, Start, Unit};
use super::filter::{Filtered, Scan, Step, step};

/// Holds graphics units whole between the query filter and the shared log.
#[derive(Debug)]
pub(super) struct GraphicsGate {
    caps: GraphicsCaps,
    scan: Scan,
    unit: Option<Unit>,
    /// A sequence whose introducer is not classified yet.
    prefix: Option<Held>,
    /// An ESC inside a string that may terminate it or begin the next sequence.
    escape: Option<Held>,
    /// Raw end of everything the gate was given.
    seen_end: u64,
    /// A Kitty series interrupted between chunks, continued by the next Kitty unit.
    series: Option<Series>,
}

impl GraphicsGate {
    pub(super) const fn new(caps: GraphicsCaps) -> Self {
        Self {
            caps,
            scan: Scan::Ground,
            unit: None,
            prefix: None,
            escape: None,
            seen_end: 0,
            series: None,
        }
    }

    /// Passes `input` on, holding a trailing graphics unit or undecided introducer.
    pub(super) fn apply(&mut self, input: &Filtered) -> Gated {
        let start = self.held_start().unwrap_or_else(|| input.raw_start.get());
        let mut out = Gated::starting_at(start);
        let mut raw = input.raw_start.get();
        let mut removed = input.removed.iter().copied().peekable();
        for (index, byte) in input.bytes.iter().copied().enumerate() {
            while let Some((_, len)) = removed.next_if(|(at, _)| *at <= index) {
                self.removal(len, &mut out);
                raw = raw.saturating_add(len);
            }
            self.byte((byte, raw), &mut out);
            raw = raw.saturating_add(1);
        }
        for (_, len) in removed {
            self.removal(len, &mut out);
        }
        self.seen_end = input.raw_end.get();
        out.filtered.raw_end = OutputOffset::new(self.held_start().unwrap_or(self.seen_end));
        out
    }

    /// Releases everything held as it is: the program's output has ended.
    pub(super) fn flush(&mut self) -> Gated {
        let mut out = Gated::starting_at(self.held_start().unwrap_or(self.seen_end));
        self.finish_unit(&mut out);
        for held in [self.prefix.take(), self.escape.take()]
            .into_iter()
            .flatten()
        {
            out.release(held);
        }
        self.scan = Scan::Ground;
        out.filtered.raw_end = OutputOffset::new(self.seen_end);
        out
    }

    fn held_start(&self) -> Option<u64> {
        self.unit
            .as_ref()
            .and_then(Unit::held_start)
            .or_else(|| self.prefix.as_ref().map(|held| held.raw_start))
            .or_else(|| self.escape.as_ref().map(|held| held.raw_start))
    }

    fn byte(&mut self, (byte, raw): (u8, u64), out: &mut Gated) {
        let (next, event) = step(self.scan, byte);
        self.scan = next;
        match event {
            Step::Begin => {
                self.settle(out);
                self.prefix = Some(Held::at(raw));
                self.route(byte, out);
            }
            Step::Continue if next == Scan::TextEscape => {
                let mut escape = Held::at(raw);
                escape.bytes.push(byte);
                self.escape = Some(escape);
            }
            Step::Continue => self.route(byte, out),
            Step::End => {
                self.release_escape(out);
                self.route(byte, out);
                self.ended(out);
            }
            Step::Abort => {
                self.release_escape(out);
                self.settle(out);
                self.finish_unit(out);
                out.push(byte);
            }
            Step::EndBeforeEscape => {
                self.ended(out);
                self.prefix = self.escape.take();
                // The byte after the terminating ESC is read again in the escape state.
                self.byte((byte, raw), out);
            }
        }
    }

    fn route(&mut self, byte: u8, out: &mut Gated) {
        if let Some(prefix) = self.prefix.as_mut() {
            prefix.bytes.push(byte);
            self.decide(out);
            return;
        }
        let caps = self.caps;
        match self.unit.as_mut() {
            Some(unit) if unit.open => unit.take(byte, (caps, out)),
            Some(_) => {
                self.finish_unit(out);
                out.push(byte);
            }
            None => out.push(byte),
        }
    }

    fn decide(&mut self, out: &mut Gated) {
        let verdict = self
            .prefix
            .as_ref()
            .map_or(Verdict::Other, |prefix| classify(&prefix.bytes));
        let protocol = match verdict {
            Verdict::Undecided => return,
            Verdict::Other => return self.settle(out),
            Verdict::Graphics(protocol) => protocol,
        };
        let Some(prefix) = self.prefix.take() else {
            return;
        };
        let caps = self.caps;
        match self.unit.as_mut() {
            Some(unit)
                if !unit.open
                    && protocol == GraphicsProtocol::Kitty
                    && unit.protocol() == GraphicsProtocol::Kitty =>
            {
                unit.next_chunk(prefix, (caps, out));
            }
            _ => {
                self.finish_unit(out);
                let series = if protocol == GraphicsProtocol::Kitty {
                    self.series.take()
                } else {
                    None
                };
                let start = Start { protocol, series };
                self.unit = Some(Unit::begin(start, prefix, (caps, out)));
            }
        }
    }

    /// The undecided introducer is another sequence: whatever came before it goes
    /// first, then its bytes.
    fn settle(&mut self, out: &mut Gated) {
        if let Some(prefix) = self.prefix.take() {
            self.finish_unit(out);
            out.release(prefix);
        }
    }

    /// The current sequence ended; a Kitty chunk announcing another keeps its unit open.
    fn ended(&mut self, out: &mut Gated) {
        if self.prefix.is_some() {
            return self.settle(out);
        }
        if let Some(unit) = self.unit.as_mut()
            && unit.open
        {
            unit.open = false;
            if !unit.awaits_chunk() {
                self.finish_unit(out);
            }
        }
    }

    fn finish_unit(&mut self, out: &mut Gated) {
        if let Some(unit) = self.unit.take() {
            if unit.protocol() == GraphicsProtocol::Kitty {
                self.series = unit.carried();
            }
            unit.finish(out);
        }
    }

    fn release_escape(&mut self, out: &mut Gated) {
        let Some(escape) = self.escape.take() else {
            return;
        };
        if let Some(prefix) = self.prefix.as_mut() {
            return prefix.append(escape);
        }
        let caps = self.caps;
        match self.unit.as_mut() {
            Some(unit) if unit.open => unit.absorb(escape, (caps, out)),
            _ => out.release(escape),
        }
    }

    fn removal(&mut self, len: u64, out: &mut Gated) {
        if let Some(held) = self.escape.as_mut().or(self.prefix.as_mut()) {
            return held.removal(len);
        }
        match self.unit.as_mut() {
            Some(unit) => unit.removal(len, out),
            None => out.discard(len),
        }
    }
}
