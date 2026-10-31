//! Mouse reports pass byte-identical when they match the negotiated tracking and encoding.

use ovis_terminal_core::input::encoding::{EncodingError, InputRequest, ModeSnapshot, encode};
use proptest::prelude::*;

use super::fixture::vt;
use super::{TestResult, rejection};

fn mouse(output: &[u8], report: &[u8]) -> TestResult<Result<Vec<u8>, EncodingError>> {
    let snapshot = ModeSnapshot::capture(&vt(output)?);
    Ok(encode(&snapshot, InputRequest::Mouse(report)).map(|encoded| encoded.bytes().to_vec()))
}

#[test]
fn reports_in_the_negotiated_encoding_are_delivered_unchanged() -> TestResult {
    let cases: [(&[u8], &[u8]); 4] = [
        (
            b"\x1b[?1000h\x1b[?1006h",
            b"\x1b[<0;1;1M\x1b[<64;200;50M\x1b[<0;1;1m",
        ),
        (b"\x1b[?1002h", b"\x1b[M !!"),
        (b"\x1b[?1003h", b"\x1b[M\xff\xff\xff"),
        (b"\x1b[?1000h\x1b[?1005h", "\x1b[M #\u{7ff}".as_bytes()),
    ];
    for (modes, report) in cases {
        assert_eq!(
            mouse(modes, report)?.as_deref().ok(),
            Some(report),
            "{report:?}"
        );
    }
    Ok(())
}

#[test]
fn reports_that_do_not_match_the_modes_are_invalid_encoding() -> TestResult {
    let off = rejection(mouse(b"", b"\x1b[<0;1;1M")?)?;
    assert!(matches!(off, EncodingError::MouseOff { .. }));
    let cases: [(&[u8], &[u8]); 6] = [
        (b"\x1b[?1000h", b"\x1b[<0;1;1M"),
        (b"\x1b[?1000h\x1b[?1006h", b"\x1b[M !!"),
        (b"\x1b[?1000h\x1b[?1006h", b"\x1b[<0;1;1Mx"),
        (b"\x1b[?1000h\x1b[?1006h", b"\x1b[<;1;1M"),
        (b"\x1b[?1000h", b"\x1b[M !"),
        (b"\x1b[?1000h", b""),
    ];
    for (modes, report) in cases {
        let error = rejection(mouse(modes, report)?)?;
        assert!(
            matches!(error.kind(), "invalid_encoding" | "invalid_params"),
            "{report:?}: {error}"
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn sgr_reports_round_trip_and_a_cut_report_is_refused(
        reports in prop::collection::vec((0u16..256, 1u16..10_000, 1u16..10_000, any::<bool>()), 1..8),
        cut in any::<prop::sample::Index>(),
    ) {
        let encoded: Vec<Vec<u8>> = reports
            .iter()
            .map(|(button, x, y, press)| {
                format!("\x1b[<{button};{x};{y}{}", if *press { 'M' } else { 'm' }).into_bytes()
            })
            .collect();
        let bytes = encoded.concat();
        let sgr = b"\x1b[?1000h\x1b[?1006h";
        let whole = mouse(sgr, &bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(whole.as_deref().ok(), Some(&bytes[..]));
        let last = encoded.last().map_or(0, Vec::len);
        let start = bytes.len().saturating_sub(last);
        let keep = start.saturating_add(1).saturating_add(cut.index(last.saturating_sub(1)));
        let truncated = mouse(sgr, bytes.get(..keep).unwrap_or_default())
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(truncated.is_err(), "a report cut inside its last bytes must be refused");
    }
}
