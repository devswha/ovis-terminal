//! Mouse reports checked against the negotiated tracking and encoding, never rewritten.

use ovis_terminal_vt::{MouseEncoding, MouseTracking};

use super::error::{EncodingError, PayloadIndex};
use super::modes::ModeSnapshot;

/// Length of one SGR report (`ESC [ < b ; x ; y M|m`) at the start of `rest`.
fn sgr(rest: &[u8]) -> Option<usize> {
    let body = rest.strip_prefix(b"\x1b[<")?;
    let mut separators = 0_u8;
    let mut digit = false;
    for (index, byte) in body.iter().enumerate() {
        match byte {
            b'0'..=b'9' => digit = true,
            b';' if digit && separators < 2 => {
                separators = separators.checked_add(1)?;
                digit = false;
            }
            b'M' | b'm' if digit && separators == 2 => return index.checked_add(4),
            _ => return None,
        }
    }
    None
}

/// Width of one X10 coordinate byte, or of one two-byte UTF-8 coordinate (1005).
fn coordinate(bytes: &[u8], utf8: bool) -> Option<usize> {
    match (utf8, *bytes.first()?) {
        (_, 0x20..=0x7f) | (false, 0x80..=0xff) => Some(1),
        (true, 0xc2..=0xdf) => matches!(bytes.get(1), Some(0x80..=0xbf)).then_some(2),
        _ => None,
    }
}

/// Length of one `ESC [ M` report with three coordinates at the start of `rest`.
fn legacy(rest: &[u8], utf8: bool) -> Option<usize> {
    let mut tail = rest.strip_prefix(b"\x1b[M")?;
    let mut length = 3_usize;
    for _ in 0..3 {
        let width = coordinate(tail, utf8)?;
        tail = tail.get(width..)?;
        length = length.checked_add(width)?;
    }
    Some(length)
}

/// Accepts `report` when it is one or more whole reports in the negotiated encoding.
pub(super) fn check(report: &[u8], snapshot: &ModeSnapshot) -> Result<(), EncodingError> {
    if report.is_empty() {
        return Err(EncodingError::Empty);
    }
    let mouse = snapshot.modes().mouse;
    if matches!(mouse.tracking, MouseTracking::Off) {
        return Err(EncodingError::MouseOff {
            revision: snapshot.revision(),
        });
    }
    let mut at = 0_usize;
    while let Some(rest) = report.get(at..).filter(|rest| !rest.is_empty()) {
        let length = match mouse.encoding {
            MouseEncoding::Sgr => sgr(rest),
            MouseEncoding::Default => legacy(rest, false),
            MouseEncoding::Utf8 => legacy(rest, true),
            _ => None,
        };
        at = length
            .and_then(|length| at.checked_add(length))
            .ok_or_else(|| EncodingError::MalformedMouse {
                at: PayloadIndex::new(at),
                expected: mouse.encoding,
            })?;
    }
    Ok(())
}
