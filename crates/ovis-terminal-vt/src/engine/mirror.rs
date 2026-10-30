//! A coarse copy of vte 0.15's parser state machine (`vte/src/lib.rs`),
//! detailed enough to know when an OSC or other string is open.
//!
//! vte keeps its state private, and its callbacks do not report when a
//! string starts, so the tracker follows the same transitions byte by byte.
//! C0 controls execute or are ignored without changing the state, except CAN
//! and SUB (back to ground) and ESC (a fresh escape); the CSI sub-states are
//! merged because all of them end at the same final bytes.

use crate::engine::tracker::StringKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum VteState {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsIgnore,
    DcsPassthrough,
    Osc,
    SosPmApc,
}

/// C0 bytes vte executes (or ignores) without leaving the current state.
const fn is_inert_c0(byte: u8) -> bool {
    matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f)
}

impl VteState {
    /// The state after vte consumes `byte` in this state.
    pub(super) const fn next(self, byte: u8) -> Self {
        match byte {
            0x18 | 0x1a => return Self::Ground,
            0x1b => return Self::Escape,
            _ => {}
        }
        match self {
            Self::Ground | Self::DcsIgnore | Self::SosPmApc => self,
            Self::Escape => Self::after_escape(byte),
            Self::EscapeIntermediate => match byte {
                0x30..=0x7e => Self::Ground,
                _ => self,
            },
            Self::Csi => match byte {
                0x40..=0x7e => Self::Ground,
                _ => self,
            },
            Self::DcsEntry | Self::DcsParam | Self::DcsIntermediate => self.dcs_header(byte),
            Self::DcsPassthrough => match byte {
                0x9c => Self::Ground,
                _ => self,
            },
            Self::Osc => match byte {
                0x07 => Self::Ground,
                _ => self,
            },
        }
    }

    const fn after_escape(byte: u8) -> Self {
        match byte {
            0x20..=0x2f => Self::EscapeIntermediate,
            0x50 => Self::DcsEntry,
            0x58 | 0x5e | 0x5f => Self::SosPmApc,
            0x5b => Self::Csi,
            0x5d => Self::Osc,
            0x30..=0x7e => Self::Ground,
            _ => Self::Escape,
        }
    }

    const fn dcs_header(self, byte: u8) -> Self {
        match (self, byte) {
            (_, 0x40..=0x7e) => Self::DcsPassthrough,
            (Self::DcsEntry | Self::DcsParam, 0x20..=0x2f) => Self::DcsIntermediate,
            (Self::DcsEntry, 0x30..=0x3f) => Self::DcsParam,
            (Self::DcsParam, 0x3c..=0x3f) | (Self::DcsIntermediate, 0x30..=0x3f) => Self::DcsIgnore,
            _ => self,
        }
    }

    /// Whether vte appends `byte` to its OSC buffer when it arrives in this
    /// state (the bytes the raw OSC bounds must count).
    pub(super) const fn stores_osc_byte(self, byte: u8) -> bool {
        matches!(self, Self::Osc)
            && !is_inert_c0(byte)
            && !matches!(byte, 0x07 | 0x18 | 0x1a | 0x1b)
    }

    pub(super) const fn string(self) -> Option<StringKind> {
        match self {
            Self::Osc => Some(StringKind::Osc),
            Self::DcsEntry
            | Self::DcsParam
            | Self::DcsIntermediate
            | Self::DcsIgnore
            | Self::DcsPassthrough
            | Self::SosPmApc => Some(StringKind::Control),
            Self::Ground | Self::Escape | Self::EscapeIntermediate | Self::Csi => None,
        }
    }
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
