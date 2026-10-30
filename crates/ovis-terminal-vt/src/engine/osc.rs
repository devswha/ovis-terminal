//! Bounded OSC records for the later agent owner (OSC 7501 and friends).
//!
//! The engine never interprets a record: it only reports what the program
//! sent, with oversize and malformed sequences as typed outcomes.

use serde::{Deserialize, Serialize};

use crate::engine::units::OutputOffset;

/// Largest OSC payload, in bytes, reported field by field.
pub const OSC_RECORD_LIMIT: usize = 4096;

/// Most fields (number included) vte keeps; it silently drops the rest.
pub const OSC_FIELD_LIMIT: usize = 16;

/// OSC numbers the emulator consumes itself (titles, colors, hyperlinks,
/// cursor shape and clipboard); they never become records, so clipboard
/// payloads are not copied out of the emulator.
const EMULATOR_OSC: [u32; 15] = [0, 1, 2, 4, 8, 10, 11, 12, 22, 50, 52, 104, 110, 111, 112];

/// How the program terminated the OSC string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OscTerminator {
    /// BEL (0x07).
    Bell,
    /// String terminator (`ESC \`).
    StringTerminator,
    /// The engine aborted the string at its parser bound; the program's own
    /// terminator, if any, comes later and is discarded.
    Truncated,
}

/// Why an OSC could not be read as `number;fields`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OscFault {
    /// The first field is empty.
    MissingNumber,
    /// The first field is not a decimal number below 2^32.
    InvalidNumber,
    /// More than [`OSC_FIELD_LIMIT`] fields; the parser cannot keep them all.
    TooManyFields,
}

/// What an OSC carried.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OscContent {
    /// A numbered OSC split at `;`.
    Fields {
        /// The OSC number.
        number: u32,
        /// Raw fields after the number.
        fields: Vec<Vec<u8>>,
    },
    /// The OSC number could not be parsed; the payload is dropped.
    Malformed {
        /// The parse failure.
        fault: OscFault,
    },
    /// The payload exceeded [`OSC_RECORD_LIMIT`]; only its size is kept.
    Oversize {
        /// The OSC number when it parsed.
        number: Option<u32>,
        /// Raw payload size in bytes, saturating at `u32::MAX`; for a
        /// [`OscTerminator::Truncated`] record, the size when it was cut.
        bytes: u32,
    },
}

/// One OSC the program sent, located in the output stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OscRecord {
    /// Offset just past the byte that completed the sequence.
    pub offset: OutputOffset,
    /// Terminator the program used.
    pub terminator: OscTerminator,
    /// Parsed content.
    pub content: OscContent,
}

fn parse_number(field: &[u8]) -> Result<u32, OscFault> {
    if field.is_empty() {
        return Err(OscFault::MissingNumber);
    }
    std::str::from_utf8(field)
        .ok()
        .filter(|text| text.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|text| text.parse().ok())
        .ok_or(OscFault::InvalidNumber)
}

/// Raw size of an OSC payload, counted byte by byte before the parser
/// splits it, so dropped fields still count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct RawOsc {
    pub(super) bytes: usize,
    pub(super) separators: usize,
}

impl RawOsc {
    pub(super) const fn push(&mut self, byte: u8) {
        self.bytes = self.bytes.saturating_add(1);
        if byte == b';' {
            self.separators = self.separators.saturating_add(1);
        }
    }

    fn oversize(self, number: Option<u32>) -> OscContent {
        OscContent::Oversize {
            number,
            bytes: u32::try_from(self.bytes).unwrap_or(u32::MAX),
        }
    }
}

/// Number of an OSC cut at the parser bound, read from its open bytes
/// (`ESC ] number ;...`); `None` when absent or emulator-owned.
pub(super) fn truncated(open: &[u8], raw: RawOsc) -> Option<OscContent> {
    let payload = open.get(2..).unwrap_or_default();
    let field = payload
        .split(|&byte| byte == b';')
        .next()
        .unwrap_or_default();
    let number = parse_number(field).ok();
    if number.is_some_and(|number| EMULATOR_OSC.contains(&number)) {
        return None;
    }
    Some(raw.oversize(number))
}

/// Classifies dispatched OSC parameters against the raw payload size;
/// `None` for emulator-owned numbers.
pub(super) fn classify(params: &[&[u8]], raw: RawOsc) -> Option<OscContent> {
    let (first, rest) = params.split_first()?;
    let number = parse_number(first);
    if number.is_ok_and(|number| EMULATOR_OSC.contains(&number)) {
        return None;
    }
    if raw.bytes > OSC_RECORD_LIMIT {
        return Some(raw.oversize(number.ok()));
    }
    if raw.separators >= OSC_FIELD_LIMIT {
        return Some(OscContent::Malformed {
            fault: OscFault::TooManyFields,
        });
    }
    Some(match number {
        Ok(number) => OscContent::Fields {
            number,
            fields: rest.iter().map(|field| field.to_vec()).collect(),
        },
        Err(fault) => OscContent::Malformed { fault },
    })
}
