//! Sequence boundaries read from raw output one byte at a time, with the attach
//! filter's rules, so a graphics sequence is keyed by the raw offset of its ESC exactly
//! as the gate keys the unit it releases.

use super::header::{Class, Header, classify};
use crate::engine::units::OutputOffset;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
/// Bytes of a sequence kept for its header; every header read here is shorter.
const HEAD_LIMIT: usize = 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    #[default]
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

const fn step(state: State, byte: u8) -> (State, Step) {
    match (state, byte) {
        (State::Ground | State::Escape | State::Csi, ESC) => (State::Escape, Step::Begin),
        (State::Ground, _) => (State::Ground, Step::Continue),
        (_, CAN | SUB) => (State::Ground, Step::Abort),
        (State::Escape, b'[') => (State::Csi, Step::Continue),
        (State::Escape, b']' | b'P' | b'X' | b'^' | b'_') => (State::Text, Step::Continue),
        (State::Escape, 0x30..=0x7e) | (State::Csi, 0x40..=0x7e) | (State::Text, BEL) => {
            (State::Ground, Step::End)
        }
        (State::Text, ESC) => (State::TextEscape, Step::Continue),
        (State::TextEscape, b'\\') => (State::Ground, Step::End),
        (State::TextEscape, _) => (State::Escape, Step::EndBeforeEscape),
        (state, _) => (state, Step::Continue),
    }
}

/// A completed sequence that matters for retained images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::engine) enum Seen {
    /// A graphics sequence that began at `key`.
    Graphics { key: OutputOffset, header: Header },
    /// RIS.
    Reset,
}

#[derive(Debug)]
struct Open {
    key: OutputOffset,
    head: Vec<u8>,
}

/// Tracks the sequence open at the stream boundary across feeds.
#[derive(Debug, Default)]
pub(in crate::engine) struct Scanner {
    state: State,
    open: Option<Open>,
}

impl Scanner {
    /// Reads the byte at raw offset `at`; returns the sequence it completed, if any.
    pub(in crate::engine) fn step(&mut self, byte: u8, at: OutputOffset) -> Option<Seen> {
        let (next, event) = step(self.state, byte);
        self.state = next;
        match event {
            Step::Begin => {
                self.open = Some(Open {
                    key: at,
                    head: Vec::new(),
                });
                self.push(byte);
                None
            }
            Step::Continue => {
                self.push(byte);
                None
            }
            Step::End => {
                self.push(byte);
                self.close()
            }
            Step::Abort => {
                self.open = None;
                None
            }
            Step::EndBeforeEscape => {
                let ended = self.close();
                self.open = Some(Open {
                    key: OutputOffset::new(at.get().saturating_sub(1)),
                    head: vec![ESC],
                });
                // The byte after the terminating ESC is read again in the escape state.
                self.step(byte, at).or(ended)
            }
        }
    }

    fn push(&mut self, byte: u8) {
        if let Some(open) = self.open.as_mut()
            && open.head.len() < HEAD_LIMIT
        {
            open.head.push(byte);
        }
    }

    fn close(&mut self) -> Option<Seen> {
        let open = self.open.take()?;
        match classify(&open.head)? {
            Class::Graphics(header) => Some(Seen::Graphics {
                key: open.key,
                header,
            }),
            Class::Reset => Some(Seen::Reset),
        }
    }
}
