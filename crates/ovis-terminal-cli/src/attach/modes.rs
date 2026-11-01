//! The outer terminal's input and screen modes the attached program switched on.
//!
//! Every byte the client draws passes through [`ModeTracker`], a small escape-sequence
//! scanner that survives any split of the stream into chunks. When the attach ends, by
//! detach, refusal, connection loss or signal, [`ModeTracker::reset`] gives the bytes
//! that switch those modes off again, so the shell the user returns to does not keep
//! mouse reporting, bracketed paste, application cursor keys, a hidden cursor, a pushed
//! keyboard protocol or the alternate screen.

use std::collections::BTreeSet;

/// DEC private modes reset on exit, alternate-screen modes last.
const RESET_ORDER: [u16; 16] = [
    1, 9, 1000, 1001, 1002, 1003, 1004, 1005, 1006, 1015, 1016, 2004, 2026, 1047, 47, 1049,
];
/// DECTCEM: visible by default, so it is reset by setting it.
const CURSOR_VISIBLE: u16 = 25;
/// Bound on CSI parameters kept per sequence; longer ones are scanned but not applied.
const MAX_PARAMS: usize = 16;
/// Bound on pushed keyboard-protocol levels counted.
const MAX_KEYBOARD_DEPTH: u16 = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Ground,
    Escape,
    Csi {
        prefix: Option<u8>,
        params: Vec<u16>,
        current: Option<u16>,
        overflow: bool,
    },
}

/// Modes outside the DEC private set that also leave the terminal changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Changed {
    /// Keyboard-protocol flags set without a push.
    KeyboardFlags,
    /// DECKPAM: the keypad sends application sequences.
    KeypadApplication,
    /// DECTCEM reset: the cursor is hidden.
    CursorHidden,
}

/// Modes left switched on by what was drawn so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ModeTracker {
    scan: Scan,
    private: BTreeSet<u16>,
    changed: BTreeSet<Changed>,
    keyboard_depth: u16,
    drew: bool,
}

fn switch(set: &mut BTreeSet<Changed>, (mode, on): (Changed, bool)) {
    if on {
        set.insert(mode);
    } else {
        set.remove(&mode);
    }
}

impl ModeTracker {
    /// Scans `bytes`, continuing any sequence an earlier chunk left open.
    pub(super) fn track(&mut self, bytes: &[u8]) {
        self.drew |= !bytes.is_empty();
        for byte in bytes {
            self.step(*byte);
        }
    }

    fn step(&mut self, byte: u8) {
        let scan = std::mem::take(&mut self.scan);
        self.scan = match (scan, byte) {
            (_, 0x1b) => Scan::Escape,
            (Scan::Ground, _) | (Scan::Csi { .. }, 0x18 | 0x1a) => Scan::Ground,
            (Scan::Escape, b'[') => Scan::Csi {
                prefix: None,
                params: Vec::new(),
                current: None,
                overflow: false,
            },
            (Scan::Escape, other) => {
                self.escape(other);
                Scan::Ground
            }
            (
                Scan::Csi {
                    prefix,
                    mut params,
                    current,
                    mut overflow,
                },
                other,
            ) => match other {
                b'<'..=b'?' if prefix.is_none() && params.is_empty() && current.is_none() => {
                    Scan::Csi {
                        prefix: Some(other),
                        params,
                        current,
                        overflow,
                    }
                }
                b'0'..=b'9' => {
                    let digit = char::from(other)
                        .to_digit(10)
                        .and_then(|digit| u16::try_from(digit).ok());
                    let next = current
                        .unwrap_or(0)
                        .checked_mul(10)
                        .zip(digit)
                        .and_then(|(value, digit)| value.checked_add(digit));
                    overflow |= next.is_none();
                    Scan::Csi {
                        prefix,
                        params,
                        current: Some(next.unwrap_or(u16::MAX)),
                        overflow,
                    }
                }
                b';' => {
                    overflow |= params.len() >= MAX_PARAMS;
                    params.push(current.unwrap_or(0));
                    Scan::Csi {
                        prefix,
                        params,
                        current: None,
                        overflow,
                    }
                }
                0x40..=0x7e => {
                    if let Some(last) = current {
                        params.push(last);
                    }
                    if !overflow {
                        self.apply((prefix, &params), other);
                    }
                    Scan::Ground
                }
                _ => Scan::Csi {
                    prefix,
                    params,
                    current,
                    overflow,
                },
            },
        };
    }

    fn escape(&mut self, byte: u8) {
        match byte {
            b'=' => switch(&mut self.changed, (Changed::KeypadApplication, true)),
            b'>' => switch(&mut self.changed, (Changed::KeypadApplication, false)),
            b'c' => {
                *self = Self {
                    drew: true,
                    ..Self::default()
                }
            }
            _ => {}
        }
    }

    fn apply(&mut self, (prefix, params): (Option<u8>, &[u16]), last: u8) {
        match (prefix, last) {
            (Some(b'?'), b'h' | b'l') => {
                let set = last == b'h';
                for mode in params {
                    if *mode == CURSOR_VISIBLE {
                        switch(&mut self.changed, (Changed::CursorHidden, !set));
                    } else if RESET_ORDER.contains(mode) {
                        if set {
                            self.private.insert(*mode);
                        } else {
                            self.private.remove(mode);
                        }
                    }
                }
            }
            (Some(b'>'), b'u') => {
                self.keyboard_depth = self
                    .keyboard_depth
                    .saturating_add(1)
                    .min(MAX_KEYBOARD_DEPTH);
            }
            (Some(b'<'), b'u') => {
                let popped = params.first().copied().unwrap_or(1).max(1);
                self.keyboard_depth = self.keyboard_depth.saturating_sub(popped);
            }
            (Some(b'='), b'u') => {
                let on = params.first().copied().unwrap_or(0) != 0;
                switch(&mut self.changed, (Changed::KeyboardFlags, on));
            }
            _ => {}
        }
    }

    /// The bytes that switch every mode left on back to its default; empty when nothing
    /// was drawn.
    pub(super) fn reset(&self) -> Vec<u8> {
        if !self.drew {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        for mode in RESET_ORDER {
            if self.private.contains(&mode) {
                bytes.extend(format!("\x1b[?{mode}l").into_bytes());
            }
        }
        if self.keyboard_depth > 0 {
            bytes.extend(format!("\x1b[<{}u", self.keyboard_depth).into_bytes());
        }
        for changed in &self.changed {
            bytes.extend_from_slice(match changed {
                Changed::KeyboardFlags => b"\x1b[=0u",
                Changed::KeypadApplication => b"\x1b>",
                Changed::CursorHidden => b"\x1b[?25h",
            });
        }
        bytes.extend(b"\x1b[0m");
        bytes
    }
}

#[cfg(test)]
mod tests;
