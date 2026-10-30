//! A second parser fed one byte at a time: it locates every completed sequence
//! in the stream, which the emulator's own parser does not expose.
//!
//! From those positions it derives the chunking-independent content revision,
//! the segment cuts that pin query replies to offsets, the OSC records, and the
//! bytes of a sequence still open at the stream boundary. It also bounds what
//! both vte parsers accumulate: vte buffers an OSC payload without limit, so an
//! open sequence that reaches [`SEQUENCE_LIMIT`] is aborted with CAN (which
//! alacritty ignores), and the rest of an aborted string is discarded up to
//! its terminator instead of reaching either parser.

use alacritty_terminal::vte::Parser;
use serde::{Deserialize, Serialize};

use crate::engine::fidelity::InexactReason;
use crate::engine::mirror::VteState;
use crate::engine::observer::{Completion, Observer};
use crate::engine::osc::{OscRecord, OscTerminator, RawOsc, truncated};
use crate::engine::units::OutputOffset;

const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;

/// Largest open sequence, in bytes, either parser or the checkpoint holds.
pub(super) const SEQUENCE_LIMIT: usize = 64 * 1024;

/// A string sequence open in the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum StringKind {
    /// OSC, ended by BEL or ST.
    Osc,
    /// DCS, SOS, PM or APC, ended by ST.
    Control,
}

/// A segment boundary: emulator bytes up to `forward_end` correspond to input
/// bytes up to `input_end`.
pub(super) struct Cut {
    pub(super) forward_end: usize,
    pub(super) input_end: usize,
}

/// What one fed chunk contained.
#[derive(Default)]
pub(super) struct Scan {
    /// Bytes the emulator must parse: the input minus discarded string bytes,
    /// plus the CAN that aborted an oversized sequence.
    pub(super) forward: Vec<u8>,
    /// Boundaries just past each completed CSI, ESC or string.
    pub(super) cuts: Vec<Cut>,
    pub(super) completions: u64,
    pub(super) osc: Vec<OscRecord>,
}

pub(super) struct Tracker {
    parser: Parser,
    pending: Vec<u8>,
    tainted: bool,
    state: VteState,
    raw: RawOsc,
    draining: Option<StringKind>,
}

impl Tracker {
    pub(super) fn new() -> Self {
        Self {
            parser: Parser::new(),
            pending: Vec::new(),
            tainted: false,
            state: VteState::Ground,
            raw: RawOsc::default(),
            draining: None,
        }
    }

    /// Rebuilds the parser state an exact checkpoint captured.
    pub(super) fn resume(pending: &[u8], draining: Option<StringKind>) -> Self {
        let mut tracker = Self::new();
        for &byte in pending {
            tracker.step(byte);
        }
        tracker.draining = draining;
        tracker
    }

    /// Bytes of the sequence open at the current boundary, or why they cannot
    /// be replayed.
    pub(super) fn open_sequence(&self) -> Result<&[u8], InexactReason> {
        if self.tainted {
            Err(InexactReason::ControlInsideOpenSequence)
        } else {
            Ok(&self.pending)
        }
    }

    /// The kind of aborted string whose remaining bytes are being discarded.
    pub(super) const fn draining(&self) -> Option<StringKind> {
        self.draining
    }

    pub(super) fn scan(&mut self, start: OutputOffset, bytes: &[u8]) -> Scan {
        let mut scan = Scan::default();
        for (index, &byte) in bytes.iter().enumerate() {
            if self.discards(byte) {
                continue;
            }
            if self.pending.len() >= SEQUENCE_LIMIT && !self.ends_sequence(byte) {
                self.abort(&mut scan, start.advanced_by(index));
                if self.draining.is_some() {
                    continue;
                }
            }
            scan.forward.push(byte);
            let observer = self.step(byte);
            scan.completions = scan.completions.saturating_add(observer.completions);
            let input_end = index.saturating_add(1);
            if let Some((terminator, content)) = observer.osc {
                let offset = start.advanced_by(input_end);
                scan.osc.push(OscRecord {
                    offset,
                    terminator,
                    content,
                });
            }
            if matches!(
                observer.last,
                Some(Completion::Ground | Completion::StringEnd { .. })
            ) {
                scan.cuts.push(Cut {
                    forward_end: scan.forward.len(),
                    input_end,
                });
            }
        }
        scan
    }

    /// Whether `byte` belongs to an aborted string and is dropped. A string
    /// ends at ST (its ESC is parsed normally), CAN or SUB, and an OSC also at
    /// BEL, which is dropped with the rest.
    fn discards(&mut self, byte: u8) -> bool {
        let Some(kind) = self.draining else {
            return false;
        };
        match byte {
            ESC | CAN | SUB => {
                self.draining = None;
                false
            }
            BEL if kind == StringKind::Osc => {
                self.draining = None;
                true
            }
            _ => true,
        }
    }

    fn ends_sequence(&self, byte: u8) -> bool {
        matches!(byte, ESC | CAN | SUB) || (byte == BEL && self.state == VteState::Osc)
    }

    /// Aborts the open sequence with CAN in both parsers, reporting an
    /// aborted OSC as a truncated oversize record.
    fn abort(&mut self, scan: &mut Scan, at: OutputOffset) {
        if self.state == VteState::Osc
            && let Some(content) = truncated(self.raw)
        {
            let terminator = OscTerminator::Truncated;
            scan.osc.push(OscRecord {
                offset: at,
                terminator,
                content,
            });
        }
        self.draining = self.state.string();
        scan.forward.push(CAN);
        let observer = self.step(CAN);
        scan.completions = scan.completions.saturating_add(observer.completions);
    }

    fn step(&mut self, byte: u8) -> Observer {
        let mut observer = Observer::new(self.raw);
        self.parser
            .advance(&mut observer, std::slice::from_ref(&byte));
        self.track_string(byte);
        self.track_boundary(observer.last, byte);
        observer
    }

    /// Follows vte's own state; raw OSC bytes are counted exactly as vte
    /// buffers them, so interleaved C0 controls cannot hide an OSC's size.
    fn track_string(&mut self, byte: u8) {
        let next = self.state.next(byte);
        if self.state.stores_osc_byte(byte) {
            self.raw.push(byte);
        } else if next == VteState::Osc && self.state != VteState::Osc {
            self.raw = RawOsc::default();
        }
        self.state = next;
    }

    fn track_boundary(&mut self, completion: Option<Completion>, byte: u8) {
        match completion {
            Some(Completion::Ground | Completion::StringEnd { bell: true }) => self.close(),
            Some(Completion::StringEnd { bell: false }) if byte != ESC => self.close(),
            Some(Completion::Control) if byte == CAN || byte == SUB => self.close(),
            Some(Completion::Control) if self.pending.is_empty() => {}
            // ESC opens a fresh escape from any state, abandoning what was open.
            Some(Completion::StringEnd { bell: false }) | None if byte == ESC => {
                self.close();
                self.pending.push(byte);
            }
            Some(Completion::Control) => {
                self.tainted = true;
                self.pending.push(byte);
            }
            Some(Completion::StringEnd { .. }) | None => self.pending.push(byte),
        }
    }

    fn close(&mut self) {
        self.pending.clear();
        self.tainted = false;
    }
}
