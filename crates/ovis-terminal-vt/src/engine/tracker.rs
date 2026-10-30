//! A second parser fed one byte at a time: it locates every completed sequence
//! in the stream, which the emulator's own parser does not expose.
//!
//! From those positions it derives the chunking-independent content revision,
//! the segment cuts that pin query replies to offsets, the OSC records, and the
//! bytes of a sequence still open at the stream boundary.

use alacritty_terminal::vte::{Params, Parser, Perform};

use crate::engine::fidelity::InexactReason;
use crate::engine::osc::{OscRecord, OscTerminator, classify};
use crate::engine::units::OutputOffset;

const ESC: u8 = 0x1b;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// Bytes of an open sequence kept for exact checkpoints.
pub(super) const PENDING_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Completion {
    /// Printable character, CSI or ESC dispatch: the parser is in ground state.
    Ground,
    /// OSC or DCS string end: ground, unless ESC ended it and opened an escape.
    StringEnd { bell: bool },
    /// C0 control executed, possibly inside an open sequence.
    Control,
}

#[derive(Default)]
struct Observer {
    completions: u64,
    last: Option<Completion>,
    osc: Option<(OscTerminator, crate::engine::osc::OscContent)>,
}

impl Observer {
    const fn complete(&mut self, completion: Completion) {
        self.completions = self.completions.saturating_add(1);
        self.last = Some(completion);
    }
}

/// Implements the vte callbacks whose signatures vte fixes at four or five
/// parameters; the observer only needs to know that they completed.
macro_rules! dispatch_observers {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) { self.complete(Completion::Ground) })*
    };
}

impl Perform for Observer {
    dispatch_observers! {
        csi_dispatch(_params: &Params, _intermediates: &[u8], _ignore: bool, _action: char);
        esc_dispatch(_intermediates: &[u8], _ignore: bool, _byte: u8);
    }

    fn print(&mut self, _: char) {
        self.complete(Completion::Ground);
    }

    fn execute(&mut self, _: u8) {
        self.complete(Completion::Control);
    }

    fn unhook(&mut self) {
        self.complete(Completion::StringEnd { bell: false });
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.complete(Completion::StringEnd {
            bell: bell_terminated,
        });
        let terminator = if bell_terminated {
            OscTerminator::Bell
        } else {
            OscTerminator::StringTerminator
        };
        self.osc = classify(params).map(|content| (terminator, content));
    }
}

/// What one fed chunk contained.
pub(super) struct Scan {
    /// Chunk-relative positions just past each completed CSI, ESC or string.
    pub(super) cuts: Vec<usize>,
    pub(super) completions: u64,
    pub(super) osc: Vec<OscRecord>,
}

pub(super) struct Tracker {
    parser: Parser,
    pending: Vec<u8>,
    inexact: Option<InexactReason>,
}

impl Tracker {
    pub(super) fn new() -> Self {
        Self {
            parser: Parser::new(),
            pending: Vec::new(),
            inexact: None,
        }
    }

    /// Rebuilds the parser state an exact checkpoint captured.
    pub(super) fn resume(pending: &[u8]) -> Self {
        let mut tracker = Self::new();
        tracker.parser.advance(&mut Observer::default(), pending);
        tracker.pending = pending.to_vec();
        tracker
    }

    /// Bytes of the sequence open at the current boundary, or why they cannot
    /// be replayed.
    pub(super) fn open_sequence(&self) -> Result<&[u8], InexactReason> {
        self.inexact.map_or(Ok(self.pending.as_slice()), Err)
    }

    pub(super) fn scan(&mut self, start: OutputOffset, bytes: &[u8]) -> Scan {
        let mut scan = Scan {
            cuts: Vec::new(),
            completions: 0,
            osc: Vec::new(),
        };
        for (index, &byte) in bytes.iter().enumerate() {
            let mut observer = Observer::default();
            self.parser
                .advance(&mut observer, std::slice::from_ref(&byte));
            scan.completions = scan.completions.saturating_add(observer.completions);
            let end = index.saturating_add(1);
            if let Some((terminator, content)) = observer.osc {
                let offset = start.advanced_by(end);
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
                scan.cuts.push(end);
            }
            self.track_boundary(observer.last, byte);
        }
        scan
    }

    fn track_boundary(&mut self, completion: Option<Completion>, byte: u8) {
        match completion {
            Some(Completion::Ground | Completion::StringEnd { bell: true }) => self.close(),
            Some(Completion::StringEnd { bell: false }) if byte != ESC => self.close(),
            Some(Completion::Control) if byte == CAN || byte == SUB => self.close(),
            Some(Completion::Control) if self.pending.is_empty() => {}
            Some(Completion::StringEnd { bell: false }) => {
                self.close();
                self.hold(byte);
            }
            Some(Completion::Control) => {
                self.inexact
                    .get_or_insert(InexactReason::ControlInsideOpenSequence);
                self.hold(byte);
            }
            None => self.hold(byte),
        }
    }

    fn close(&mut self) {
        self.pending.clear();
        self.inexact = None;
    }

    fn hold(&mut self, byte: u8) {
        if self.pending.len() < PENDING_LIMIT {
            self.pending.push(byte);
        } else {
            self.inexact = Some(InexactReason::OpenSequenceTooLong);
        }
    }
}
