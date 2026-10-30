//! Bounded OSC records for the later agent owner (OSC 7501 and friends).
//!
//! The engine never interprets a record: it only reports what the program
//! sent, with oversize and malformed sequences as typed outcomes.

use serde::{Deserialize, Serialize};

use crate::engine::units::OutputOffset;

/// Largest OSC payload, in bytes, reported field by field.
pub const OSC_RECORD_LIMIT: usize = 4096;

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
        /// Payload size in bytes, saturating at `u32::MAX`.
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

/// Classifies dispatched OSC parameters; `None` for emulator-owned numbers.
pub(super) fn classify(params: &[&[u8]]) -> Option<OscContent> {
    let (first, rest) = params.split_first()?;
    let number = parse_number(first);
    if number.is_ok_and(|number| EMULATOR_OSC.contains(&number)) {
        return None;
    }
    let bytes = params
        .iter()
        .map(|field| field.len())
        .sum::<usize>()
        .saturating_add(params.len().saturating_sub(1));
    if bytes > OSC_RECORD_LIMIT {
        return Some(OscContent::Oversize {
            number: number.ok(),
            bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
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
